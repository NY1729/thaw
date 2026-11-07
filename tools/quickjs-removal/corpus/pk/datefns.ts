import { addDays, addHours, addMinutes, differenceInHours, differenceInMinutes, fromUnixTime, getUnixTime, isAfter, isValid, parseISO } from "date-fns";
// Only instant-based helpers and toISOString() are used: no local-zone formatting (format/startOfDay) so output is TZ-independent.
// addDays is only used on Jan/Feb 2024, before any DST transition in common zones.
function line(label: string, v: unknown): void {
  console.log(label + ":" + (typeof v === "string" ? v : JSON.stringify(v)));
}
async function main(): Promise<void> {
  const base = parseISO("2024-02-28T10:30:00Z");
  line("addDays", addDays(base, 2).toISOString());
  line("addHours", addHours(base, 36).toISOString());
  line("addMinutes", addMinutes(base, -90).toISOString());
  line("diffHours", differenceInHours(parseISO("2024-03-01T00:00:00Z"), base));
  line("diffMinutes", differenceInMinutes(parseISO("2024-03-01T00:00:00Z"), base));
  line("isAfter", isAfter(base, parseISO("2024-01-01T00:00:00Z")));
  line("invalid", isValid(parseISO("nope")));
  line("unix", getUnixTime(base));
  line("fromUnix", fromUnixTime(1709116200).toISOString());
  line("offset", parseISO("2024-02-28T12:30:00+02:00").toISOString());
  // WORKLOAD: N=4000 (estimate; date-fns calls are ~35 us each under Node). Numeric fold over minute differences.
  const N = 4000;
  let acc = 0;
  for (let i = 0; i < N; i++) {
    acc = (acc * 31 + String(differenceInMinutes(addMinutes(base, i * 7), base) + (i % 13)).length + (i % 11)) | 0;
  }
  line("checksum", acc);
}
