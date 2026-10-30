// Run after installing the official validator locally under verification/gltf-validator.
// node scripts/verify_glb.cjs verification/exports
const fs = require('node:fs');
const path = require('node:path');
const root = path.resolve(process.argv[2] || 'verification/exports');
const validator = require(require.resolve('gltf-validator', { paths: [path.resolve('verification/gltf-validator')] }));
(async () => {
  const manifest = JSON.parse(fs.readFileSync(path.join(root, 'manifest.json'), 'utf8'));
  const cases = [];
  for (const item of manifest.cases) {
    const report = await validator.validateBytes(new Uint8Array(fs.readFileSync(path.join(root, item.file))), { uri: item.file, maxIssues: 100 });
    fs.writeFileSync(path.join(root, `${item.name}-validator.json`), JSON.stringify(report, null, 2));
    cases.push({ name: item.name, errors: report.issues.numErrors, warnings: report.issues.numWarnings, validator: report.validatorVersion });
  }
  const passed = cases.every(item => item.errors === 0 && item.warnings === 0);
  fs.writeFileSync(path.join(root, 'validator-report.json'), JSON.stringify({ passed, cases }, null, 2));
  console.log(JSON.stringify({ passed, cases }, null, 2));
  if (!passed) process.exitCode = 1;
})().catch(error => { console.error(error); process.exitCode = 1; });
