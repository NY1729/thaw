// Typed inventory service: generics, interfaces, enums, discriminated unions, Map, JSON snapshot.
const Category = { Tool: 'tool', Food: 'food' } as const;
type Category = (typeof Category)[keyof typeof Category];

interface Item {
  sku: string;
  name: string;
  category: Category;
  qty: number;
  price: number;
}

type Event =
  | { kind: 'add'; item: Item }
  | { kind: 'remove'; sku: string; qty: number }
  | { kind: 'reprice'; sku: string; price: number };

class Repo<T extends { sku: string }> {
  private items = new Map<string, T>();
  upsert(t: T): void {
    this.items.set(t.sku, t);
  }
  get(sku: string): T | undefined {
    return this.items.get(sku);
  }
  all(): T[] {
    return [...this.items.values()].sort((a, b) => a.sku.localeCompare(b.sku));
  }
}

class Inventory {
  readonly repo = new Repo<Item>();
  readonly log: string[] = [];
  apply(e: Event): void {
    switch (e.kind) {
      case 'add': {
        const cur = this.repo.get(e.item.sku);
        this.repo.upsert(cur ? { ...cur, qty: cur.qty + e.item.qty } : e.item);
        break;
      }
      case 'remove': {
        const cur = this.repo.get(e.sku);
        if (!cur || cur.qty < e.qty) {
          this.log.push(`cannot remove ${e.qty} of ${e.sku}`);
          return;
        }
        this.repo.upsert({ ...cur, qty: cur.qty - e.qty });
        break;
      }
      case 'reprice':
        this.repo.upsert({ ...this.repo.get(e.sku)!, price: e.price });
        break;
    }
    this.log.push(e.kind + ':' + ('sku' in e ? e.sku : e.item.sku));
  }
  value(): number {
    return this.repo.all().reduce((s, i) => s + i.qty * i.price, 0);
  }
}

const inv = new Inventory();
const events: Event[] = [
  { kind: 'add', item: { sku: 'H1', name: 'hammer', category: Category.Tool, qty: 5, price: 12.5 } },
  { kind: 'add', item: { sku: 'A1', name: 'apple', category: Category.Food, qty: 100, price: 0.4 } },
  { kind: 'remove', sku: 'A1', qty: 30 },
  { kind: 'remove', sku: 'H1', qty: 9 },
  { kind: 'reprice', sku: 'H1', price: 15 },
  { kind: 'add', item: { sku: 'H1', name: 'hammer', category: Category.Tool, qty: 2, price: 15 } },
];
for (const e of events) inv.apply(e);
console.log(JSON.stringify(inv.repo.all()));
console.log(inv.log, inv.value().toFixed(2), Category.Tool, Object.values(Category));
