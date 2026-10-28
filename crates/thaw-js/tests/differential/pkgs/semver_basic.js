// pkg: semver
const semver = require('semver');
const log = (...a) => console.log(...a.map((x) => (typeof x === 'string' ? x : JSON.stringify(x))));
log(semver.valid('1.2.3'), semver.valid('1.2'), semver.clean('  =v1.2.3  '), semver.coerce('v2.5 beta').version);
log(semver.gt('1.2.3', '1.2.0'), semver.lt('1.2.3', '1.10.0'), semver.eq('1.0.0', '1.0.0+build'), semver.compare('2.0.0', '10.0.0'), semver.rcompare('1.0.0', '1.0.1'));
log(semver.inc('1.2.3', 'patch'), semver.inc('1.2.3', 'minor'), semver.inc('1.2.3', 'major'), semver.inc('1.2.3', 'prerelease', 'beta'), semver.inc('1.2.3-beta.1', 'prerelease'));
log(semver.satisfies('1.2.3', '^1.0.0'), semver.satisfies('2.0.0', '^1.0.0'), semver.satisfies('1.5.0', '1.x || >=3'), semver.satisfies('1.2.3-beta.2', '>=1.2.3-beta.1 <1.3.0'), semver.satisfies('0.2.5', '~0.2.1'));
log(semver.maxSatisfying(['1.0.0', '1.5.0', '2.0.0', '1.9.9'], '^1.0.0'), semver.minSatisfying(['1.0.0', '1.5.0', '2.0.0'], '>1.0.0'), semver.sort(['1.10.0', '1.2.0', '1.9.1']), semver.rsort(['1.10.0', '1.2.0', '1.9.1']));
log(semver.major('4.5.6'), semver.minor('4.5.6'), semver.patch('4.5.6'), semver.prerelease('1.0.0-alpha.1'), semver.diff('1.0.0', '1.1.0'), semver.diff('1.0.0', '2.0.0-rc'));
log(semver.validRange('^1.2.3'), semver.validRange('nonsense'), semver.intersects('^1.0.0', '>=1.5.0'), semver.gtr('2.0.0', '^1.0.0'), semver.ltr('0.1.0', '^1.0.0'), semver.subset('^1.2.3', '^1.0.0'));
const v = new semver.SemVer('3.4.5-rc.1+exp');
log(v.major, v.minor, v.patch, v.prerelease, v.build, v.version, v.raw, String(v), v.compare('3.4.5'));
const r = new semver.Range('>=1.2.3 <2.0.0 || ^3.0.0');
log(r.range, r.test('1.5.0'), r.test('2.5.0'), r.test('3.1.0'), r.set.length);
try { new semver.SemVer('not a version'); } catch (e) { log(e.name, e.message); }
log(semver.parse('1.2.3-alpha').prerelease, semver.parse('garbage'), semver.minVersion('>=1.2.3').version, semver.toComparators('1.2.x'));
