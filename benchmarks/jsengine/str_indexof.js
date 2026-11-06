// indexOf loop over a prebuilt 300 KB ASCII string, stepping 60 units per call; expected checksum: str_indexof 5000 299943
const s = "ab cd ".repeat(50000);
let count = 0;
let last = -1;
let pos = s.indexOf("cd");
while (pos !== -1) {
  count++;
  last = pos;
  pos = s.indexOf("cd", pos + 60);
}
console.log("str_indexof " + count + " " + last);
