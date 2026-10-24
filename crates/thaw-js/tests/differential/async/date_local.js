const d = new Date(2020, 0, 31, 13, 45, 6, 789);
console.log(d.getFullYear(), d.getMonth(), d.getDate(), d.getDay(), d.getHours(), d.getMinutes(), d.getSeconds(), d.getMilliseconds(), d.getTimezoneOffset());
const nz = (s) => s.replace(/ \(.*\)$/, '');
console.log(d.toISOString(), d.getTime(), nz(d.toString()), d.toDateString(), nz(d.toTimeString()));
console.log(d.toUTCString(), d.toJSON(), JSON.stringify({ d }));
const e = new Date(d); e.setMonth(1); console.log(e.toISOString());
e.setDate(0); console.log(e.toISOString()); e.setHours(25, 61, 61, 1001); console.log(e.toISOString());
e.setFullYear(2021, 11, 31); console.log(e.toISOString(), e.setUTCHours(3), e.getUTCHours());
e.setMinutes(90); e.setSeconds(-1); e.setMilliseconds(5000); console.log(e.toISOString());
console.log(new Date(2020, 11, 31, 23, 59, 59).getTime(), new Date(99, 0).getFullYear(), new Date(0, 0).getFullYear());
console.log(Date.UTC(2020, 1, 29), Date.UTC(99, 0), new Date(2024, 1, 30).getDate());
// parsing
for (const s of ['2020-01-02', '2020-01-02T03:04:05', '2020-01-02T03:04:05Z', '2020-01-02T03:04:05+09:00', '2020-01-02T03:04:05.123-0530', '2020-01-02 03:04:05', '2020-01', '2020',
  'Thu, 01 Jan 1970 00:00:00 GMT', 'Jan 2, 2020', 'January 2, 2020 10:30:00', '2 Jan 2020', '1/2/2020', '12/31/1999 11:59 PM', '2020/01/02', 'Tue Mar 01 2022 10:00:00 GMT+0100', 'nonsense', '', '2020-13-01', '2020-02-30T00:00:00Z']) {
  console.log(JSON.stringify(s), Date.parse(s));
}
// DST edges (meaningful in zones with DST)
for (const [y, m, dd, h] of [[2021, 2, 14, 2], [2021, 2, 14, 3], [2021, 10, 7, 1], [2021, 2, 28, 2], [2021, 9, 31, 2], [2021, 3, 4, 2]]) {
  const t = new Date(y, m, dd, h, 30);
  console.log(y, m, dd, h, t.getTime(), t.getHours(), t.getTimezoneOffset());
}
for (const t of [0, 1e12, 1615705200000, 1636264800000, -1e12, 253402300799000]) { const x = new Date(t); console.log(t, x.getHours(), x.getDate(), x.getTimezoneOffset(), nz(x.toString())); }
console.log(new Date(NaN).getTime(), String(new Date(NaN)), new Date(8.64e15 + 1).getTime(), isNaN(new Date('x')), new Date(null).getTime(), new Date(2020, 0).getMonth());
const inv = new Date(NaN); inv.setFullYear(2000); console.log(inv.getFullYear(), inv.getMonth(), new Date(NaN).setHours(1));
console.log(new Date(2020, 5, 15, 8, 5, 9).toLocaleDateString('en-US'), new Date(2020, 5, 15, 18, 5, 9).toLocaleTimeString('en-US'), new Date(2020, 5, 15, 0, 5, 9).toLocaleString('en-US'));
console.log(new Date(Date.UTC(2020, 5, 15, 8, 5, 9)).toLocaleString('en-US', { timeZone: 'UTC' }));
