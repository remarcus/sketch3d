//! Client for the optional remote model sharing service.
//!
//! Capabilities are deliberately kept out of returned metadata and URLs. The only
//! persistent storage implementation in this module is the operating system
//! credential store; callers can inject an in-memory implementation in tests.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use ureq::http::Uri;

const KEYRING_SERVICE: &str = "sketch3d.share-service";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_RESPONSE_BYTES: u64 = 64 * 1024;
/// Guard against accidentally uploading a very large export in one request.
pub const MAX_SHARE_BYTES: usize = 10 * 1024 * 1024;
const MODEL_ID_LEN: usize = 48;

/// Errors returned by the sharing client. Values intentionally omit server bodies,
/// request URLs, and credentials so they are safe to display to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShareError {
    InvalidServiceUrl,
    InvalidModel,
    InvalidModelId,
    InvalidResponse,
    HttpStatus(u16),
    Network,
    CapabilityUnavailable,
    CapabilityStore,
}

impl fmt::Display for ShareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidServiceUrl => f.write_str("The sharing service URL is invalid."),
            Self::InvalidModel => f.write_str("The GLB model is invalid or too large to share."),
            Self::InvalidModelId => f.write_str("The shared model identifier is invalid."),
            Self::InvalidResponse => {
                f.write_str("The sharing service returned an invalid response.")
            }
            Self::HttpStatus(code) => write!(f, "The sharing service returned HTTP {code}."),
            Self::Network => f.write_str("Could not complete the sharing service request."),
            Self::CapabilityUnavailable => {
                f.write_str("No saved sharing capability is available for this model.")
            }
            Self::CapabilityStore => {
                f.write_str("The operating system credential store is unavailable.")
            }
        }
    }
}

impl std::error::Error for ShareError {}

/// Minimal secure storage interface, injectable to make client behavior testable.
pub trait CapabilityStore: Send + Sync {
    fn get(&self, model_id: &str) -> Result<Option<String>, ShareError>;
    fn put(&self, model_id: &str, capability: &str) -> Result<(), ShareError>;
    fn delete(&self, model_id: &str) -> Result<(), ShareError>;
}

/// Capability storage backed by the platform keyring (Windows Credential Manager,
/// macOS Keychain, or an available Linux Secret Service provider).
#[derive(Debug, Default, Clone, Copy)]
pub struct OsCapabilityStore;

impl OsCapabilityStore {
    fn entry(model_id: &str) -> Result<keyring::Entry, ShareError> {
        validate_model_id(model_id)?;
        keyring::Entry::new(KEYRING_SERVICE, model_id).map_err(|_| ShareError::CapabilityStore)
    }
}

impl CapabilityStore for OsCapabilityStore {
    fn get(&self, model_id: &str) -> Result<Option<String>, ShareError> {
        let entry = Self::entry(model_id)?;
        match entry.get_secret() {
            Ok(secret) => String::from_utf8(secret)
                .map(Some)
                .map_err(|_| ShareError::CapabilityStore),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(ShareError::CapabilityStore),
        }
    }

    fn put(&self, model_id: &str, capability: &str) -> Result<(), ShareError> {
        let entry = Self::entry(model_id)?;
        entry
            .set_secret(capability.as_bytes())
            .map_err(|_| ShareError::CapabilityStore)
    }

    fn delete(&self, model_id: &str) -> Result<(), ShareError> {
        let entry = Self::entry(model_id)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(ShareError::CapabilityStore),
        }
    }
}

/// Public data for a shared model. The bearer capability is never returned here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedArtwork {
    pub model_id: String,
    pub viewer_url: String,
    pub expires_at: String,
    pub bytes: u64,
}

/// Result of replacing a share. The service does not repeat the public URL or
/// management capability in its update response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatedArtwork {
    pub model_id: String,
    pub expires_at: String,
    pub bytes: u64,
}

/// Blocking client for the versioned model sharing API.
pub struct ShareServiceClient {
    origin: String,
    agent: ureq::Agent,
    capabilities: Arc<dyn CapabilityStore>,
}

impl ShareServiceClient {
    pub fn new(origin: &str, capabilities: Arc<dyn CapabilityStore>) -> Result<Self, ShareError> {
        let origin = validate_service_origin(origin)?;
        let agent: ureq::Agent = ureq::config::Config::builder()
            .max_redirects(0)
            .timeout_global(Some(REQUEST_TIMEOUT))
            .build()
            .into();
        Ok(Self {
            origin,
            agent,
            capabilities,
        })
    }

    /// Uploads the exact supplied GLB bytes and stores the returned capability in
    /// the injected credential store. If secure storage fails, it attempts to
    /// revoke the newly created share before returning an error.
    pub fn publish(&self, glb: &[u8]) -> Result<SharedArtwork, ShareError> {
        validate_glb(glb)?;
        let url = format!("{}/api/v1/models", self.origin);
        let response = self
            .agent
            .post(url)
            .header("Content-Type", "model/gltf-binary")
            .send(glb)
            .map_err(map_http_error)?;
        let created = read_json::<CreateResponse>(response)?;
        validate_model_id(&created.id)?;
        validate_capability(&created.capability)?;
        let metadata =
            validate_metadata(created.id, created.url, created.expires_at, created.bytes)?;
        if self
            .capabilities
            .put(&metadata.model_id, &created.capability)
            .is_err()
        {
            // Best effort cleanup. Do not expose or persist the capability if the
            // OS credential store rejected it.
            let _ = self.delete_with_capability(&metadata.model_id, &created.capability);
            return Err(ShareError::CapabilityStore);
        }
        Ok(metadata)
    }

    /// Replaces the GLB bytes for an existing share using its saved capability.
    pub fn update(&self, model_id: &str, glb: &[u8]) -> Result<UpdatedArtwork, ShareError> {
        validate_model_id(model_id)?;
        validate_glb(glb)?;
        let capability = self
            .capabilities
            .get(model_id)?
            .ok_or(ShareError::CapabilityUnavailable)?;
        validate_capability(&capability)?;
        let url = format!("{}/api/v1/models/{model_id}", self.origin);
        let response = self
            .agent
            .put(url)
            .header("Content-Type", "model/gltf-binary")
            .header("Authorization", format!("Bearer {capability}"))
            .send(glb)
            .map_err(map_http_error)?;
        let updated = read_json::<UpdateResponse>(response)?;
        if updated.id != model_id {
            return Err(ShareError::InvalidResponse);
        }
        Ok(UpdatedArtwork {
            model_id: updated.id,
            expires_at: updated.expires_at.to_string(),
            bytes: updated.bytes,
        })
    }

    /// Revokes an existing share and removes its capability from secure storage.
    pub fn unpublish(&self, model_id: &str) -> Result<(), ShareError> {
        validate_model_id(model_id)?;
        let capability = self
            .capabilities
            .get(model_id)?
            .ok_or(ShareError::CapabilityUnavailable)?;
        validate_capability(&capability)?;
        self.delete_with_capability(model_id, &capability)?;
        self.capabilities.delete(model_id)
    }

    fn delete_with_capability(&self, model_id: &str, capability: &str) -> Result<(), ShareError> {
        let url = format!("{}/api/v1/models/{model_id}", self.origin);
        self.agent
            .delete(url)
            .header("Authorization", format!("Bearer {capability}"))
            .call()
            .map(|_| ())
            .map_err(map_http_error)
    }
}

/// Validates and canonicalizes an API origin. Only HTTPS is accepted, except
/// loopback hosts used by local development and tests. Paths, credentials,
/// queries, and fragments are rejected.
pub fn validate_service_origin(input: &str) -> Result<String, ShareError> {
    if input.trim() != input || input.contains('#') || input.contains('?') {
        return Err(ShareError::InvalidServiceUrl);
    }
    let uri: Uri = input.parse().map_err(|_| ShareError::InvalidServiceUrl)?;
    let scheme = uri.scheme_str().ok_or(ShareError::InvalidServiceUrl)?;
    let authority = uri.authority().ok_or(ShareError::InvalidServiceUrl)?;
    let host = authority.host();
    if host.is_empty() || authority.as_str().contains('@') {
        return Err(ShareError::InvalidServiceUrl);
    }
    let is_loopback =
        host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "[::1]";
    if !(scheme.eq_ignore_ascii_case("https")
        || (scheme.eq_ignore_ascii_case("http") && is_loopback))
    {
        return Err(ShareError::InvalidServiceUrl);
    }
    if uri.path_and_query().is_some_and(|path| path.path() != "/") {
        return Err(ShareError::InvalidServiceUrl);
    }
    Ok(format!("{}://{}", scheme.to_ascii_lowercase(), authority))
}

fn validate_glb(glb: &[u8]) -> Result<(), ShareError> {
    if glb.len() < 12
        || glb.len() > MAX_SHARE_BYTES
        || &glb[..4] != b"glTF"
        || u32::from_le_bytes([glb[4], glb[5], glb[6], glb[7]]) != 2
        || u32::from_le_bytes([glb[8], glb[9], glb[10], glb[11]]) as usize != glb.len()
    {
        return Err(ShareError::InvalidModel);
    }
    Ok(())
}

fn validate_model_id(model_id: &str) -> Result<(), ShareError> {
    if model_id.len() != MODEL_ID_LEN
        || !model_id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(ShareError::InvalidModelId);
    }
    Ok(())
}

fn validate_capability(capability: &str) -> Result<(), ShareError> {
    if capability.is_empty()
        || capability.len() > 4096
        || !capability.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'+' | b'/' | b'=')
        })
    {
        return Err(ShareError::InvalidResponse);
    }
    Ok(())
}

fn validate_metadata(
    model_id: String,
    viewer_url: String,
    expires_at: u64,
    bytes: u64,
) -> Result<SharedArtwork, ShareError> {
    validate_model_id(&model_id)?;
    if bytes > MAX_SHARE_BYTES as u64 {
        return Err(ShareError::InvalidResponse);
    }
    let uri: Uri = viewer_url
        .parse()
        .map_err(|_| ShareError::InvalidResponse)?;
    let scheme = uri.scheme_str().ok_or(ShareError::InvalidResponse)?;
    let authority = uri.authority().ok_or(ShareError::InvalidResponse)?;
    let local_http = scheme.eq_ignore_ascii_case("http")
        && (authority.host().eq_ignore_ascii_case("localhost")
            || authority.host() == "127.0.0.1"
            || authority.host() == "[::1]");
    if !(scheme.eq_ignore_ascii_case("https") || local_http)
        || authority.as_str().contains('@')
        || viewer_url.len() > 2048
    {
        return Err(ShareError::InvalidResponse);
    }
    Ok(SharedArtwork {
        model_id,
        viewer_url,
        expires_at: expires_at.to_string(),
        bytes,
    })
}

#[derive(Deserialize)]
struct CreateResponse {
    #[serde(alias = "modelId")]
    id: String,
    #[serde(alias = "viewerUrl")]
    url: String,
    capability: String,
    #[serde(
        rename = "expiresAt",
        alias = "expires_at",
        deserialize_with = "deserialize_u64"
    )]
    expires_at: u64,
    bytes: u64,
}

#[derive(Deserialize)]
struct UpdateResponse {
    #[serde(alias = "modelId")]
    id: String,
    #[serde(
        rename = "expiresAt",
        alias = "expires_at",
        deserialize_with = "deserialize_u64"
    )]
    expires_at: u64,
    bytes: u64,
}

fn deserialize_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum NumberOrString {
        Number(u64),
        String(String),
    }
    match NumberOrString::deserialize(deserializer)? {
        NumberOrString::Number(number) => Ok(number),
        NumberOrString::String(value) => value.parse().map_err(serde::de::Error::custom),
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(
    mut response: ureq::http::Response<ureq::Body>,
) -> Result<T, ShareError> {
    let text = response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_to_string()
        .map_err(|_| ShareError::InvalidResponse)?;
    serde_json::from_str(&text).map_err(|_| ShareError::InvalidResponse)
}

fn map_http_error(error: ureq::Error) -> ShareError {
    match error {
        ureq::Error::StatusCode(code) => ShareError::HttpStatus(code),
        _ => ShareError::Network,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Mutex;
    use std::thread;
    use std::time::Instant;

    #[derive(Default)]
    struct MockStore(Mutex<HashMap<String, String>>);

    impl CapabilityStore for MockStore {
        fn get(&self, model_id: &str) -> Result<Option<String>, ShareError> {
            Ok(self.0.lock().unwrap().get(model_id).cloned())
        }
        fn put(&self, model_id: &str, capability: &str) -> Result<(), ShareError> {
            self.0
                .lock()
                .unwrap()
                .insert(model_id.to_owned(), capability.to_owned());
            Ok(())
        }
        fn delete(&self, model_id: &str) -> Result<(), ShareError> {
            self.0.lock().unwrap().remove(model_id);
            Ok(())
        }
    }

    fn glb() -> Vec<u8> {
        let mut bytes = b"glTF".to_vec();
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&12u32.to_le_bytes());
        bytes
    }

    fn respond(stream: &mut std::net::TcpStream, status: &str, body: &str) {
        write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    }

    fn accept(listener: &TcpListener) -> std::net::TcpStream {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    return stream;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "timed out waiting for client request"
                    );
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("could not accept client request: {error}"),
            }
        }
    }

    fn read_request(stream: &mut std::net::TcpStream) -> Vec<u8> {
        let mut request = Vec::new();
        let mut buffer = [0; 1024];
        let mut expected_len = None;
        let mut is_chunked = false;
        loop {
            let read = stream.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if expected_len.is_none() {
                if let Some(header_end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    is_chunked = headers.lines().any(|line| {
                        line.to_ascii_lowercase()
                            .starts_with("transfer-encoding: chunked")
                    });
                    let content_len = headers.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    });
                    if let Some(content_len) = content_len {
                        expected_len = Some(header_end + 4 + content_len);
                    } else if !is_chunked {
                        expected_len = Some(header_end + 4);
                    }
                }
            }
            if expected_len.is_some_and(|len| request.len() >= len)
                || (is_chunked && request.windows(5).any(|window| window == b"0\r\n\r\n"))
            {
                break;
            }
        }
        request
    }

    #[test]
    fn validates_service_origins() {
        assert_eq!(
            validate_service_origin("https://share.example.test/").unwrap(),
            "https://share.example.test"
        );
        assert_eq!(
            validate_service_origin("http://localhost:8080").unwrap(),
            "http://localhost:8080"
        );
        for bad in [
            "http://share.example.test",
            "https://user:pass@example.test",
            "https://example.test/path",
            "https://example.test?x=1",
            "https://example.test/#fragment",
        ] {
            assert_eq!(
                validate_service_origin(bad),
                Err(ShareError::InvalidServiceUrl)
            );
        }
    }

    #[test]
    fn publish_update_delete_use_exact_bytes_and_bearer_capability() {
        const MODEL_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            let mut create = accept(&listener);
            let request = read_request(&mut create);
            let received = String::from_utf8_lossy(&request);
            assert!(received.contains("POST /api/v1/models HTTP/1.1"));
            assert!(received
                .to_ascii_lowercase()
                .contains("content-type: model/gltf-binary"));
            assert!(request
                .windows(12)
                .any(|window| window == b"glTF\x02\x00\x00\x00\x0c\x00\x00\x00"));
            respond(
                &mut create,
                "201 Created",
                &format!(
                    r#"{{"id":"{MODEL_ID}","url":"https://viewer.example.test/share-viewer/?model=%2Fmodels%2F{MODEL_ID}.glb","capability":"secret_cap","expiresAt":1893456000000,"bytes":12}}"#
                ),
            );

            let mut update = accept(&listener);
            let request = read_request(&mut update);
            let received = String::from_utf8_lossy(&request);
            assert!(received.contains(&format!("PUT /api/v1/models/{MODEL_ID} HTTP/1.1")));
            assert!(received
                .to_ascii_lowercase()
                .contains("authorization: bearer secret_cap"));
            assert!(!received.contains("secret_cap?"));
            respond(
                &mut update,
                "200 OK",
                &format!(r#"{{"id":"{MODEL_ID}","bytes":12,"expiresAt":1893456000000}}"#),
            );

            let mut delete = accept(&listener);
            let request = read_request(&mut delete);
            let received = String::from_utf8_lossy(&request);
            assert!(received.contains(&format!("DELETE /api/v1/models/{MODEL_ID} HTTP/1.1")));
            assert!(received
                .to_ascii_lowercase()
                .contains("authorization: bearer secret_cap"));
            respond(&mut delete, "204 No Content", "");
        });
        let store = Arc::new(MockStore::default());
        let client = ShareServiceClient::new(&format!("http://{addr}"), store.clone()).unwrap();
        let metadata = client.publish(&glb()).unwrap();
        assert_eq!(metadata.model_id, MODEL_ID);
        assert!(!format!("{metadata:?}").contains("secret_cap"));
        assert_eq!(store.get(MODEL_ID).unwrap().as_deref(), Some("secret_cap"));
        let update = client.update(MODEL_ID, &glb()).unwrap();
        assert_eq!(update.model_id, metadata.model_id);
        assert_eq!(update.bytes, 12);
        client.unpublish(MODEL_ID).unwrap();
        assert_eq!(store.get(MODEL_ID).unwrap(), None);
        worker.join().unwrap();
    }

    #[test]
    fn redirects_are_not_followed() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            let mut stream = accept(&listener);
            let mut request = [0; 2048];
            let _ = stream.read(&mut request).unwrap();
            respond(&mut stream, "302 Found", r#"{"modelId":"x"}"#);
        });
        let client =
            ShareServiceClient::new(&format!("http://{addr}"), Arc::new(MockStore::default()))
                .unwrap();
        assert!(client.publish(&glb()).is_err());
        worker.join().unwrap();
    }
}
