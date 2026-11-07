import { DateTime, Duration } from "luxon";
// All times are fixed ISO instants in UTC; no Settings.defaultZone / local zone or system clock is read.
function line(label: string, v: unknown): void {
  console.log(label + ":" + (typeof v === "string" ? v : JSON.stringify(v)));
}
async function main(): Promise<void> {
  const d = DateTime.fromISO("2024-02-28T10:30:00.000Z", { zone: "utc" });
  line("iso", d.toISO());
  line("plusDays", d.plus({ days: 2 }).toISO());
  line("format", d.toFormat("yyyy-MM-dd HH:mm"));
  line("startMonth", d.startOf("month").toISO());
  line("fixedOffset", d.setZone("UTC+2").toISO());
  line("diffHours", DateTime.fromISO("2024-03-01T00:00:00Z", { zone: "utc" }).diff(d, "hours").hours);
  line("weekday", d.weekday);
  line("invalid", DateTime.fromISO("not-a-date", { zone: "utc" }).isValid);
  line("duration", Duration.fromObject({ hours: 1, minutes: 30 }).toISO());
  // WORKLOAD: N=3000 (estimate). Folds ordinal/weekday/hour numerically; the template's string-length fold is constant here.
  const N = 3000;
  const base = Date.UTC(2024, 0, 1);
  let acc = 0;
  for (let i = 0; i < N; i++) {
    const dt = DateTime.fromMillis(base + i * 3600000, { zone: "utc" }).plus({ days: i % 400 });
    acc = (acc * 31 + dt.ordinal * 7 + dt.weekday + dt.hour) | 0;
  }
  line("checksum", acc);
}
