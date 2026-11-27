// Subset of test/common/fixtures.js (path / readSync / fileURL / fixturesDir).
import path from 'path';
import fs from 'fs';
import { fileURLToPath, pathToFileURL } from 'url';

const fixturesDir: string = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'fixtures');

function fixturesPath(...args: string[]): string {
  return path.join(fixturesDir, ...args);
}

function fixturesFileURL(...args: string[]): any {
  return pathToFileURL(fixturesPath(...args));
}

function readFixtureSync(args: any, enc?: any): any {
  return fs.readFileSync(fixturesPath(String(args)), enc);
}

export { fixturesDir, fixturesPath as path, fixturesFileURL as fileURL, readFixtureSync as readSync };
export default { fixturesDir, path: fixturesPath, fileURL: fixturesFileURL, readSync: readFixtureSync };
