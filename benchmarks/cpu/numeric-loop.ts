function main(): void {
  let checksum: number = 0;
  for (let index: number = 0; index < 30000000; index++) {
    checksum += index % 1000;
  }
  console.log(checksum);
}
