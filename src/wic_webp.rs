//! Windows Imaging Component fallback for embedded WebP images.

#[cfg(windows)]
pub fn decode(bytes: &[u8]) -> Result<crate::image_planes::ImageAsset, String> {
    use std::ptr::{null, null_mut};
    use winapi::{
        shared::{guiddef::GUID, winerror::RPC_E_CHANGED_MODE, wtypesbase::CLSCTX_INPROC_SERVER},
        um::{
            combaseapi::{CoCreateInstance, CoInitializeEx, CoUninitialize},
            objbase::COINIT_MULTITHREADED,
            wincodec::{
                CLSID_WICImagingFactory, GUID_WICPixelFormat32bppRGBA, IWICBitmapDecoder,
                IWICBitmapFrameDecode, IWICFormatConverter, IWICImagingFactory, IWICStream,
                WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnLoad,
            },
        },
    };

    const IID_IWIC_IMAGING_FACTORY: GUID = GUID {
        Data1: 0xec5ec8a9,
        Data2: 0xc395,
        Data3: 0x4314,
        Data4: [0x9c, 0x77, 0x54, 0xd7, 0xa9, 0x35, 0xff, 0x70],
    };

    struct ComApartment(bool);
    impl Drop for ComApartment {
        fn drop(&mut self) {
            if self.0 {
                unsafe { CoUninitialize() };
            }
        }
    }

    struct ComOwned<T>(*mut T);
    impl<T> Drop for ComOwned<T> {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    (*(self.0.cast::<winapi::um::unknwnbase::IUnknown>())).Release();
                }
            }
        }
    }

    if bytes.is_empty() || bytes.len() > u32::MAX as usize {
        return Err("Embedded WebP byte length is invalid".into());
    }
    unsafe {
        let init = CoInitializeEx(null_mut(), COINIT_MULTITHREADED);
        if init < 0 && init != RPC_E_CHANGED_MODE {
            return Err(format!(
                "Cannot initialize Windows image decoding: HRESULT {init:#x}"
            ));
        }
        let _apartment = ComApartment(init >= 0);

        let mut factory = null_mut::<IWICImagingFactory>();
        let hr = CoCreateInstance(
            &CLSID_WICImagingFactory,
            null_mut(),
            CLSCTX_INPROC_SERVER,
            &IID_IWIC_IMAGING_FACTORY,
            (&mut factory as *mut *mut IWICImagingFactory).cast(),
        );
        if hr < 0 || factory.is_null() {
            return Err(format!(
                "Windows has no WebP image decoder (HRESULT {hr:#x})"
            ));
        }
        let factory = ComOwned(factory);

        let mut source = bytes.to_vec();
        let mut stream = null_mut::<IWICStream>();
        let hr = (*factory.0).CreateStream(&mut stream);
        if hr < 0 || stream.is_null() {
            return Err(format!(
                "Cannot create an image decode stream (HRESULT {hr:#x})"
            ));
        }
        let stream = ComOwned(stream);
        let hr = (*stream.0).InitializeFromMemory(source.as_mut_ptr().cast(), source.len() as u32);
        if hr < 0 {
            return Err(format!("Cannot read embedded WebP data (HRESULT {hr:#x})"));
        }

        let mut decoder = null_mut::<IWICBitmapDecoder>();
        let hr = (*factory.0).CreateDecoderFromStream(
            stream.0.cast(),
            null(),
            WICDecodeMetadataCacheOnLoad,
            &mut decoder,
        );
        if hr < 0 || decoder.is_null() {
            return Err(format!(
                "Windows could not decode this WebP image (HRESULT {hr:#x})"
            ));
        }
        let decoder = ComOwned(decoder);

        let mut frame = null_mut::<IWICBitmapFrameDecode>();
        let hr = (*decoder.0).GetFrame(0, &mut frame);
        if hr < 0 || frame.is_null() {
            return Err(format!(
                "Cannot read the first WebP frame (HRESULT {hr:#x})"
            ));
        }
        let frame = ComOwned(frame);

        let mut converter = null_mut::<IWICFormatConverter>();
        let hr = (*factory.0).CreateFormatConverter(&mut converter);
        if hr < 0 || converter.is_null() {
            return Err(format!(
                "Cannot create the WebP pixel converter (HRESULT {hr:#x})"
            ));
        }
        let converter = ComOwned(converter);
        let hr = (*converter.0).Initialize(
            frame.0.cast(),
            &GUID_WICPixelFormat32bppRGBA,
            WICBitmapDitherTypeNone,
            null(),
            0.0,
            WICBitmapPaletteTypeCustom,
        );
        if hr < 0 {
            return Err(format!(
                "Cannot convert WebP pixels to RGBA (HRESULT {hr:#x})"
            ));
        }

        let mut width = 0;
        let mut height = 0;
        let hr = (*converter.0).GetSize(&mut width, &mut height);
        let pixels = u64::from(width).saturating_mul(u64::from(height));
        if hr < 0 || width == 0 || height == 0 || pixels > crate::image_planes::MAX_IMAGE_PIXELS {
            return Err("WebP dimensions exceed the 12 megapixel image limit".into());
        }
        let stride = width.checked_mul(4).ok_or("WebP row stride is too large")?;
        let byte_len = (stride as usize)
            .checked_mul(height as usize)
            .ok_or("WebP decoded image is too large")?;
        let mut pixels = vec![0; byte_len];
        let hr = (*converter.0).CopyPixels(null(), stride, byte_len as u32, pixels.as_mut_ptr());
        if hr < 0 {
            return Err(format!("Cannot read decoded WebP pixels (HRESULT {hr:#x})"));
        }
        Ok(crate::image_planes::ImageAsset {
            size: [width, height],
            pixels,
        })
    }
}

#[cfg(not(windows))]
pub fn decode(_bytes: &[u8]) -> Result<crate::image_planes::ImageAsset, String> {
    Err("WebP decoding is unavailable on this platform".into())
}
