import dayjs from "dayjs";
function main(): void {
  const d = dayjs("2024-02-29T12:00:00Z");
  console.log(d.add(1, "year").format("YYYY-MM-DD"), d.diff(dayjs("2024-01-01T00:00:00Z"), "day"), d.isValid(), dayjs("garbage").isValid());
}
