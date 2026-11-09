// Char-by-char scans of long strings through s.length, s.charCodeAt(i) and s[i]. The ASCII
// variant is 200k chars; the mixed variant has one 'é' and one astral char near the end and is
// 20k chars (the non-ASCII path is O(n) per index today, so it is kept short).
// expected checksum: str_scan 20300100 2142522 19999 1999
const ascii = "abcdefghij".repeat(19999) + "klmnopqrst";
let sum = 0;
for (let i = 0; i < ascii.length; i++) sum += ascii.charCodeAt(i);
let aHits = 0;
for (let i = 0; i < ascii.length; i++) if (ascii[i] === "a") aHits++;

const mixed = "abcdefghij".repeat(1999) + "é😀klmnopqrst";
let msum = 0;
for (let i = 0; i < mixed.length; i++) msum += mixed.charCodeAt(i);
let mHits = 0;
for (let i = 0; i < mixed.length; i++) if (mixed[i] === "a") mHits++;

console.log("str_scan " + sum + " " + msum + " " + aHits + " " + mHits);
