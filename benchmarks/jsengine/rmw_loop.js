// read-modify-write loop over 3e6 iterations (i++, x += n, x = x op n); expected checksum: rmw_loop 4499998500000 3000000
let s = 0;
let t = 0;
for (let i = 0; i < 3000000; i++) {
  s += i;
  t = t + 2;
  t -= 1;
}
console.log("rmw_loop " + s + " " + t);
