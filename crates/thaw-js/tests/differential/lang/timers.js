const values = [];
const never = setTimeout(() => values.push('never'), 5);
clearTimeout(never);
const imm = setImmediate(() => values.push('never-immediate'));
clearImmediate(imm);
setTimeout(() => {
  values.push('timeout');
  setImmediate(() => {
    values.push('immediate');
    let count = 0;
    const timer = setInterval(() => {
      count += 1;
      values.push('tick' + count);
      if (count === 2) {
        clearInterval(timer);
        setTimeout(() => console.log(values.join(',')), 20);
      }
    }, 1);
  });
}, 1);
