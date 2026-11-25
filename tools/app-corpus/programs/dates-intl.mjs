// Date/Intl formatting under TZ=UTC: DateTimeFormat, NumberFormat, RelativeTimeFormat, PluralRules, ListFormat.
const d = new Date(Date.UTC(2024, 1, 29, 13, 5, 9, 123));
console.log(d.toISOString(), d.getTime(), d.toUTCString());
console.log(JSON.stringify(d), d.getDay(), d.getTimezoneOffset());
console.log(new Intl.DateTimeFormat('en-US', { dateStyle: 'full', timeStyle: 'short', timeZone: 'UTC' }).format(d));
console.log(new Intl.DateTimeFormat('en-GB', { year: 'numeric', month: 'long', day: '2-digit', timeZone: 'UTC' }).format(d));
console.log(new Intl.DateTimeFormat('en-US', { weekday: 'short', hour: 'numeric', minute: '2-digit', hour12: false, timeZone: 'America/New_York' }).format(d));
console.log(new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD' }).format(1234567.891));
console.log(new Intl.NumberFormat('en-US', { style: 'percent', maximumFractionDigits: 1 }).format(0.4567));
console.log(new Intl.NumberFormat('en-US', { notation: 'compact' }).format(1234567));
console.log((1234567.891).toLocaleString('en-US'), (0.5).toLocaleString('en-US', { minimumFractionDigits: 2 }));
const rtf = new Intl.RelativeTimeFormat('en', { numeric: 'auto' });
console.log(rtf.format(-1, 'day'), rtf.format(3, 'hour'), rtf.format(0, 'week'));
const pr = new Intl.PluralRules('en-US');
console.log([0, 1, 2].map((n) => pr.select(n)));
console.log(new Intl.ListFormat('en', { style: 'long', type: 'conjunction' }).format(['a', 'b', 'c']));
const addDays = (dt, n) => new Date(dt.getTime() + n * 86400000);
console.log(addDays(d, 1).toISOString().slice(0, 10), new Date('2024-03-10T00:00:00Z') - d);
console.log(new Date('not a date').getTime(), String(new Date(NaN)));
