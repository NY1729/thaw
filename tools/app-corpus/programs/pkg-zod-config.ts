// pkgs: zod@4.6.7
// Config validation with zod: defaults, coercion, refinements, error reporting.
import { z } from 'zod';

const Config = z.object({
  port: z.coerce.number().int().min(1).max(65535).default(3000),
  host: z.string().default('localhost'),
  mode: z.enum(['dev', 'prod']),
  tags: z.array(z.string()).default([]),
  limits: z.object({ rps: z.number().positive() }).optional(),
});

for (const input of [{ mode: 'dev' }, { mode: 'prod', port: '8080', tags: ['a'], limits: { rps: 5 } }, { mode: 'x', port: 0 }]) {
  const r = Config.safeParse(input);
  if (r.success) console.log('ok', JSON.stringify(r.data));
  else console.log('bad', r.error.issues.map((i) => `${i.path.join('.')}:${i.code}`).join(','));
}
