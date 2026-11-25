import dayjs from "dayjs";
// default import (dayjs is default-only). UTC only: ISO strings with Z, hour/minute units only
// (day/month arithmetic is local-calendar and would be TZ-sensitive). N = 1408 (calibrated, see ms.ts).
function line(label: string, v: unknown): void {
  console.log(label + ":" + (typeof v === "string" ? v : JSON.stringify(v)));
}
async function main(): Promise<void> {
  const N = 1408;
  const base = dayjs("2024-02-28T12:00:00Z");
  line("add36h", base.add(36, "hour").toISOString());
  line("unix", dayjs.unix(1700000000).toISOString());
  line("diffMin", dayjs("2024-03-01T00:30:00Z").diff(base, "minute"));
  line("before", base.isBefore(dayjs("2025-01-01T00:00:00Z")));
  line("valueOf", base.valueOf());
  let acc = 0;
  for (let i = 0; i < N; i++) {
    const d = dayjs("2024-01-01T00:00:00Z").add(i, "minute");
    acc = (acc * 31 + String(d.valueOf()).length + d.hour()) | 0;
  }
  line("checksum", acc);
}
