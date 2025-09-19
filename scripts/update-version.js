#!/usr/bin/env node

import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

// Read version from version.json
const versionFile = path.join(__dirname, '..', 'version.json');
const versionData = JSON.parse(fs.readFileSync(versionFile, 'utf8'));
const { version, name, description } = versionData;

console.log(`Updating all files to version ${version}...`);

// Update package.json
const packageJsonPath = path.join(__dirname, '..', 'package.json');
const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, 'utf8'));
packageJson.version = version;
fs.writeFileSync(packageJsonPath, JSON.stringify(packageJson, null, 2) + '\n');
console.log(`✅ Updated ${packageJsonPath}`);

// Update Cargo.toml
const cargoTomlPath = path.join(__dirname, '..', 'src-tauri', 'Cargo.toml');
let cargoToml = fs.readFileSync(cargoTomlPath, 'utf8');
cargoToml = cargoToml.replace(/^version = ".*"$/m, `version = "${version}"`);
cargoToml = cargoToml.replace(/^description = ".*"$/m, `description = "${description}"`);
fs.writeFileSync(cargoTomlPath, cargoToml);
console.log(`✅ Updated ${cargoTomlPath}`);

// Update tauri.conf.json files
const tauriConfigPaths = [
  path.join(__dirname, '..', 'tauri.conf.json'),
  path.join(__dirname, '..', 'src-tauri', 'tauri.conf.json')
];

for (const configPath of tauriConfigPaths) {
  if (fs.existsSync(configPath)) {
    const config = JSON.parse(fs.readFileSync(configPath, 'utf8'));
    config.version = version;
    config.productName = name;
    fs.writeFileSync(configPath, JSON.stringify(config, null, 2) + '\n');
    console.log(`✅ Updated ${configPath}`);
  }
}

// Update App.tsx version display
const appTsxPath = path.join(__dirname, '..', 'src', 'App.tsx');
let appTsx = fs.readFileSync(appTsxPath, 'utf8');
appTsx = appTsx.replace(/v\d+\.\d+\.\d+/g, `v${version}`);
fs.writeFileSync(appTsxPath, appTsx);
console.log(`✅ Updated ${appTsxPath}`);

console.log(`\n🎉 All files updated to version ${version}!`);
console.log('\n📋 Files updated:');
console.log('  - package.json');
console.log('  - src-tauri/Cargo.toml');
console.log('  - tauri.conf.json');
console.log('  - src-tauri/tauri.conf.json');
console.log('  - src/App.tsx');