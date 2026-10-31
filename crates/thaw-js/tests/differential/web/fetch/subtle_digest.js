const hex = b => [...new Uint8Array(b)].map(x => x.toString(16).padStart(2, '0')).join('');
(async () => {
  console.log(hex(await crypto.subtle.digest('SHA-256', new TextEncoder().encode('abc'))));
  console.log(hex(await crypto.subtle.digest({ name: 'SHA-1' }, new TextEncoder().encode('abc'))));
  console.log(hex(await crypto.subtle.digest('SHA-384', new Uint8Array(0))).length);
  try { await crypto.subtle.digest('MD5', new Uint8Array(1)); } catch (e) { console.log(e.name); }
})();
