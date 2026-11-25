// pkgs: semver@7.8.5 @types/semver@7.8.0
// Release tooling over semver: sorting, ranges, bumping.
import semver from 'semver';

const versions = ['1.2.3', '1.10.0', '1.2.10', '2.0.0-beta.1', '2.0.0', '0.9.9'];
console.log(semver.sort([...versions]));
console.log(semver.maxSatisfying(versions, '^1.2.0'), semver.minSatisfying(versions, '>=1.2.5'));
console.log(semver.inc('1.2.3', 'minor'), semver.inc('1.2.3', 'prerelease', 'rc'), semver.inc('2.0.0-beta.1', 'major'));
console.log(semver.valid('v1.2.3'), semver.valid('nope'), semver.coerce('release-4.5')?.version);
console.log(semver.diff('1.2.3', '1.3.0'), semver.gt('1.10.0', '1.9.9'), semver.satisfies('1.2.3', '~1.2.0 || >=3'));
