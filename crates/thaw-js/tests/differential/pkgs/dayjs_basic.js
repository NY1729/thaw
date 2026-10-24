// pkg: dayjs
const dayjs = require('dayjs');
const base = dayjs('2020-03-15T10:20:30.456');
const fmts = ['YYYY-MM-DD HH:mm:ss.SSS', 'YY M D H m s', 'MMM MMMM ddd dddd', 'DD/MM/YYYY hh:mm A a', 'Z ZZ', '[Today is] dddd', 'X x', 'd', 'Do'];
for (const f of fmts) console.log(f, '=>', base.format(f));
console.log(base.format(), base.toISOString(), base.toJSON(), base.valueOf(), base.unix(), String(base.toDate()).slice(0, 24));
for (const u of ['year', 'month', 'week', 'day', 'hour', 'minute', 'second', 'millisecond']) {
  console.log(u, base.startOf(u).format('YYYY-MM-DD HH:mm:ss.SSS'), base.endOf(u).format('YYYY-MM-DD HH:mm:ss.SSS'), base.add(3, u).valueOf(), base.subtract(2, u).valueOf());
}
console.log(base.year(), base.month(), base.date(), base.day(), base.hour(), base.minute(), base.second(), base.millisecond(), base.daysInMonth(), base.isValid());
console.log(base.set('year', 2000).format(), base.set('month', 13).format(), base.year(1999).format(), base.date(31).month(1).format());
const other = dayjs('2021-07-04T23:59:59');
for (const u of ['year', 'month', 'week', 'day', 'hour', 'minute', 'second', 'millisecond']) console.log('diff', u, other.diff(base, u), other.diff(base, u, true).toFixed(4), base.diff(other, u));
console.log(base.isBefore(other), base.isAfter(other), base.isSame(base.clone()), base.isSame(other, 'year'), base.isSame(dayjs('2020-03-01'), 'month'), dayjs.isDayjs(base), dayjs.isDayjs({}));
console.log(dayjs('2020-02-30').format(), dayjs('garbage').isValid(), dayjs('garbage').format(), dayjs(null).isValid(), dayjs(undefined).isValid(), dayjs(0).valueOf(), dayjs(1e12).toISOString(), dayjs(new Date(2020, 0, 1)).format(), dayjs([2020, 0, 5]).format?.call(dayjs([2020, 0, 5])));
console.log(dayjs('2020-03-15').format(), dayjs('20200315').format(), dayjs('2020-03-15 10:00').format(), dayjs('2020/03/15').format(), dayjs('2020-03-15T10:00:00Z').toISOString(), dayjs('2020-03-15T10:00:00+09:00').toISOString(), dayjs('March 15, 2020').isValid());
console.log(dayjs.unix(1584267630).toISOString(), dayjs('2020-12-31').add(1, 'day').format(), dayjs('2020-01-31').add(1, 'month').format(), dayjs('2020-02-29').add(1, 'year').format(), dayjs('2019-12-31').week === undefined);
console.log(base.utcOffset(), base.locale(), dayjs.locale(), base.toObject ? JSON.stringify(base.toObject()) : '-', JSON.stringify(base.toArray ? base.toArray() : '-'), base.$d instanceof Date, base.$y, base.$M);
console.log(dayjs.min === undefined, typeof dayjs.extend, typeof dayjs.en, dayjs().isValid(), Math.abs(dayjs().valueOf() - Date.now()) < 1000, Object.prototype.toString.call(base), base + '' === base.toString(), base.toString());
console.log(base.add(1.5, 'day').format(), base.add(-1, 'month').format(), base.startOf('d').format(), base.endOf('M').format(), base.add({}, 'day') === undefined);
