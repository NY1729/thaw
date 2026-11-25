// Idle-game balance simulator: seeded PRNG, big numeric loops, Map/Set, sort, JSON output.
function mulberry32(a) {
  return function () {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const rand = mulberry32(20240229);
const generators = [
  { id: 'miner', baseCost: 15, growth: 1.15, rate: 0.1 },
  { id: 'farm', baseCost: 100, growth: 1.17, rate: 1 },
  { id: 'factory', baseCost: 1100, growth: 1.2, rate: 8 },
  { id: 'lab', baseCost: 12000, growth: 1.22, rate: 47 },
];
const owned = new Map(generators.map((g) => [g.id, 0]));
const achievements = new Set();
let gold = 10;
let totalEarned = 0;
const cost = (g) => Math.floor(g.baseCost * Math.pow(g.growth, owned.get(g.id)));
const income = () => generators.reduce((s, g) => s + owned.get(g.id) * g.rate, 0);

const history = [];
for (let tick = 1; tick <= 20000; tick++) {
  const bonus = rand() < 0.01 ? 1 + Math.floor(rand() * 50) : 0;
  const gain = income() * 0.1 + bonus;
  gold += gain;
  totalEarned += gain;
  // buy the best rate-per-cost generator we can afford
  let best = null;
  for (const g of generators) {
    const c = cost(g);
    if (c <= gold && (!best || g.rate / c > best.g.rate / best.c)) best = { g, c };
  }
  if (best) {
    gold -= best.c;
    owned.set(best.g.id, owned.get(best.g.id) + 1);
  }
  for (const [id, n] of owned) if (n >= 10 && !achievements.has(`${id}10`)) achievements.add(`${id}10`);
  if (tick % 2500 === 0) history.push({ tick, gold: Math.round(gold), income: +income().toFixed(2) });
}
const ranking = [...owned.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
console.log(JSON.stringify({ history, ranking, achievements: [...achievements].sort(), totalEarned: Math.round(totalEarned) }));
