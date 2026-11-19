const hex = (b) => [...new Uint8Array(b)].map((x) => x.toString(16).padStart(2, '0')).join('');
const fails = (p) => p.then((v) => 'ok ' + (v && v.type ? 'key ' + v.type : v instanceof ArrayBuffer ? 'bytes ' + v.byteLength : String(v)), (e) => e.name + ' ' + JSON.stringify(e.message));
const enc = (s) => new TextEncoder().encode(s);
const pbkdf2 = (password, salt, iterations, hash, bits) => crypto.subtle.importKey('raw', enc(password), 'PBKDF2', false, ['deriveBits'])
  .then((k) => crypto.subtle.deriveBits({ name: 'PBKDF2', hash, salt: enc(salt), iterations }, k, bits))
  .then(hex);
(async () => {
  // RFC 6070 (SHA-1) and RFC 7914 section 11 (SHA-256) vectors
  console.log(await pbkdf2('password', 'salt', 1, 'SHA-1', 160));
  console.log(await pbkdf2('password', 'salt', 2, 'SHA-1', 160));
  console.log(await pbkdf2('password', 'salt', 4096, 'SHA-1', 160));
  console.log(await pbkdf2('passwordPASSWORDpassword', 'saltSALTsaltSALTsaltSALTsaltSALTsalt', 4096, 'SHA-1', 200));
  console.log(await pbkdf2('passwd', 'salt', 1, 'SHA-256', 512));
  console.log(await pbkdf2('Password', 'NaCl', 80000, 'SHA-256', 256).catch((e) => 'slow ' + e.name));
  console.log(await pbkdf2('password', 'salt', 2, 'sha-384', 384));
  console.log(await pbkdf2('password', 'salt', 2, { name: 'SHA-512' }, 512));
  console.log(await pbkdf2('pencil', 'QSXCR+Q6sek8bf92', 3, 'SHA-256', 128));
  // pg SCRAM-SHA-256 shape: SaltedPassword, ClientKey = HMAC(SP, "Client Key"), StoredKey = SHA-256(ClientKey)
  const salt = Buffer.from('QSXCR+Q6sek8bf92', 'base64');
  const key = await crypto.subtle.importKey('raw', enc('pencil'), 'PBKDF2', false, ['deriveBits']);
  const salted = await crypto.subtle.deriveBits({ name: 'PBKDF2', hash: 'SHA-256', salt, iterations: 4096 }, key, 256);
  console.log('SaltedPassword', hex(salted));
  const hmacKey = await crypto.subtle.importKey('raw', salted, { name: 'HMAC', hash: { name: 'SHA-256' } }, false, ['sign']);
  const clientKey = await crypto.subtle.sign('HMAC', hmacKey, enc('Client Key'));
  console.log('ClientKey', hex(clientKey));
  console.log('StoredKey', hex(await crypto.subtle.digest('SHA-256', clientKey)));
  console.log('ServerKey', hex(await crypto.subtle.sign('HMAC', hmacKey, enc('Server Key'))));
  // importKey: extractable, usages, format, data and name
  console.log(await fails(crypto.subtle.importKey('raw', enc('pw'), 'PBKDF2', true, ['deriveBits'])));
  console.log(await fails(crypto.subtle.importKey('raw', enc('pw'), { name: 'PBKDF2' }, true, ['deriveKey'])));
  console.log(await fails(crypto.subtle.importKey('raw', enc('pw'), 'PBKDF2', false, ['sign'])));
  console.log(await fails(crypto.subtle.importKey('raw', enc('pw'), 'PBKDF2', false, [])));
  console.log(await fails(crypto.subtle.importKey('raw', enc('pw'), 'PBKDF2', false, ['deriveBits', 'encrypt'])));
  console.log(await fails(crypto.subtle.importKey('jwk', { kty: 'oct', k: 'AQID' }, 'PBKDF2', false, ['deriveBits'])));
  console.log(await fails(crypto.subtle.importKey('raw', 'pw', 'PBKDF2', false, ['deriveBits'])));
  console.log(await fails(crypto.subtle.importKey('raw', enc('pw'), 'Foo', false, ['deriveBits'])));
  console.log(await fails(crypto.subtle.importKey('raw', enc('pw'), 'PBKDF2', false, ['deriveKey'])));
  const pk = await crypto.subtle.importKey('raw', enc('pw'), 'PBKDF2', 0, ['deriveBits', 'deriveKey']);
  console.log(pk.type, JSON.stringify(pk.algorithm), pk.extractable, pk.usages.join(','), Object.keys(pk).length);
  // deriveBits: hash, iterations, salt, key and length
  const salt1 = enc('salt');
  const d = (params, base, bits) => fails(crypto.subtle.deriveBits(params, base, bits));
  const good = { name: 'PBKDF2', hash: 'SHA-256', salt: salt1, iterations: 2 };
  console.log(await d({ ...good, hash: 'sha-256' }, pk, 256));
  console.log(await d({ ...good, hash: 'SHA256' }, pk, 256));
  console.log(await d({ ...good, hash: 'MD5' }, pk, 128));
  console.log(await d({ ...good, hash: undefined }, pk, 128));
  console.log(await d({ ...good, iterations: 0 }, pk, 128));
  console.log(await d({ ...good, iterations: -1 }, pk, 128));
  console.log(await d({ ...good, iterations: NaN }, pk, 128));
  console.log(await d({ ...good, iterations: 2.9 }, pk, 128));
  console.log(await d({ ...good, iterations: undefined }, pk, 128));
  console.log(await d({ ...good, salt: 'salt' }, pk, 128));
  console.log(await d({ name: 'PBKDF2', hash: 'SHA-256', iterations: 2 }, pk, 128));
  console.log(await d(good, pk, null));
  console.log(await d(good, pk, undefined));
  console.log(await d(good, pk, 7));
  console.log(await d(good, pk, 8.7));
  console.log(await d(good, pk, -8));
  console.log(await d(good, null, 128));
  console.log(await d(good, await crypto.subtle.importKey('raw', enc('pw'), 'PBKDF2', false, ['deriveKey']), 128));
  console.log(await d(good, await crypto.subtle.importKey('raw', enc('pw'), { name: 'HMAC', hash: 'SHA-256' }, true, ['sign']), 128));
  console.log(await d({ name: 'PBKDF2' }, pk, 128));
  console.log(await d(good, pk, 0));
  // deriveKey to HMAC and AES
  const dk = (type, usages, extractable = true) => crypto.subtle.deriveKey(good, pk, type, extractable, usages);
  const k1 = await dk({ name: 'HMAC', hash: 'SHA-256', length: 128 }, ['sign']);
  console.log(JSON.stringify(k1.algorithm), k1.type, k1.usages.join(','), hex(await crypto.subtle.exportKey('raw', k1)));
  const k2 = await dk({ name: 'HMAC', hash: 'SHA-1' }, ['sign', 'verify']);
  console.log(JSON.stringify(k2.algorithm), (await crypto.subtle.exportKey('raw', k2)).byteLength);
  const k3 = await dk({ name: 'AES-GCM', length: 256 }, ['encrypt', 'decrypt']);
  console.log(JSON.stringify(k3.algorithm), k3.usages.join(','), hex(await crypto.subtle.exportKey('raw', k3)));
  console.log(await fails(dk({ name: 'HMAC', hash: 'SHA-256', length: 0 }, ['sign'])));
  console.log(await fails(dk({ name: 'HMAC', hash: 'SHA-256', length: 100 }, ['sign'])));
  console.log(await fails(dk({ name: 'HMAC' }, ['sign'])));
  console.log(await fails(dk({ name: 'AES-GCM' }, ['encrypt'])));
  console.log(await fails(dk({ name: 'AES-GCM', length: 100 }, ['encrypt'])));
  console.log(await fails(dk({ name: 'AES-CBC', length: 128 }, ['sign'])));
  console.log(await fails(dk({ name: 'Foo' }, ['sign'])));
  console.log(await fails(dk({ name: 'HMAC', hash: 'SHA-256', length: 128 }, [])));
  console.log(await fails(crypto.subtle.deriveKey(good, await crypto.subtle.importKey('raw', enc('pw'), 'PBKDF2', false, ['deriveBits']), { name: 'HMAC', hash: 'SHA-256' }, true, ['sign'])));
  console.log(await fails(crypto.subtle.deriveKey(good, null, { name: 'HMAC', hash: 'SHA-256' }, true, ['sign'])));
  // HMAC sign: usage and key type
  const hk = await crypto.subtle.importKey('raw', enc('key'), { name: 'HMAC', hash: 'SHA-256' }, true, ['sign']);
  console.log(hex(await crypto.subtle.sign('HMAC', hk, enc('The quick brown fox'))));
  const hv = await crypto.subtle.importKey('raw', enc('key'), { name: 'HMAC', hash: 'SHA-256' }, true, ['verify']);
  console.log(await fails(crypto.subtle.sign('HMAC', hv, enc('x'))));
  console.log(await fails(crypto.subtle.sign('HMAC', pk, enc('x'))));
  console.log(await fails(crypto.subtle.sign('HMAC', hk, 'x')));
  console.log(await fails(crypto.subtle.exportKey('raw', pk)));
})();
