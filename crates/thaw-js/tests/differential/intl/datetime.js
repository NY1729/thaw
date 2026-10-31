const d = new Date(Date.UTC(2024, 0, 15, 13, 45, 30));
console.log(new Intl.DateTimeFormat('en-US', { timeZone: 'UTC' }).format(d));
console.log(new Intl.DateTimeFormat('en-US', { dateStyle: 'full', timeStyle: 'short', timeZone: 'America/New_York' }).format(d));
console.log(new Intl.DateTimeFormat('de-DE', { dateStyle: 'long', timeZone: 'Europe/Berlin' }).format(d));
console.log(JSON.stringify(new Intl.DateTimeFormat('en-US', { year: 'numeric', month: 'short', day: '2-digit', timeZone: 'UTC' }).formatToParts(d)));
const ro = new Intl.DateTimeFormat('ja-JP', { timeZone: 'Asia/Tokyo' }).resolvedOptions();
console.log(ro.locale, ro.timeZone, ro.calendar);
console.log(new Intl.DateTimeFormat('en-US', { timeZone: 'UTC', month: 'long' }).format(d));
