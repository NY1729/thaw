  if (typeof globalThis.URLSearchParams !== 'function') {
    const encodeFormPart = value => encodeURIComponent(String(value)).replace(/[!'()~]/g, character => '%' + character.charCodeAt(0).toString(16).toUpperCase()).replace(/%20/g, '+');
    const decodeFormPart = value => {
      const source = new TextEncoder().encode(String(value).replace(/\+/g, ' '));
      const bytes = [];
      const hex = byte => byte >= 48 && byte <= 57 ? byte - 48
        : byte >= 65 && byte <= 70 ? byte - 55 : byte >= 97 && byte <= 102 ? byte - 87 : -1;
      for (let index = 0; index < source.length; index++) {
        const high = hex(source[index + 1]), low = hex(source[index + 2]);
        if (source[index] === 37 && high >= 0 && low >= 0) {
          bytes.push(high * 16 + low); index += 2;
        } else bytes.push(source[index]);
      }
      return new TextDecoder('utf-8', { ignoreBOM: true }).decode(Uint8Array.from(bytes));
    };
    globalThis.URLSearchParams = class URLSearchParams {
      constructor(init = '') {
        this.__thawEntries = [];
        this.__thawUpdate = null;
        if (typeof init === 'string') {
          const source = init.charAt(0) === '?' ? init.substring(1) : init;
          if (source !== '') for (const field of source.split('&')) {
            if (field === '') continue;
            const separator = field.indexOf('=');
            const name = separator < 0 ? field : field.substring(0, separator);
            const value = separator < 0 ? '' : field.substring(separator + 1);
            this.append(decodeFormPart(name), decodeFormPart(value));
          }
        } else if (init != null && typeof init[Symbol.iterator] === 'function') {
          for (const pair of init) {
            if (pair == null || typeof pair[Symbol.iterator] !== 'function') throw new TypeError('URLSearchParams entry must be a pair');
            const values = [...pair];
            if (values.length !== 2) throw new TypeError('URLSearchParams entry must contain two values');
            this.append(values[0], values[1]);
          }
        } else if (init != null && typeof init === 'object') {
          for (const name of Object.keys(init)) this.append(name, init[name]);
        }
      }
      get size() { return this.__thawEntries.length; }
      __thawChanged() { if (this.__thawUpdate) this.__thawUpdate(this.toString()); }
      append(name, value) {
        this.__thawEntries.push([String(name), String(value)]);
        this.__thawChanged();
      }
      delete(name, value) {
        const key = String(name);
        if (arguments.length < 2) this.__thawEntries = this.__thawEntries.filter(entry => entry[0] !== key);
        else {
          const expected = String(value);
          this.__thawEntries = this.__thawEntries.filter(entry => entry[0] !== key || entry[1] !== expected);
        }
        this.__thawChanged();
      }
      get(name) {
        const key = String(name);
        const entry = this.__thawEntries.find(candidate => candidate[0] === key);
        return entry ? entry[1] : null;
      }
      getAll(name) {
        const key = String(name);
        return this.__thawEntries.filter(entry => entry[0] === key).map(entry => entry[1]);
      }
      has(name, value) {
        const key = String(name);
        if (arguments.length < 2) return this.__thawEntries.some(entry => entry[0] === key);
        const expected = String(value);
        return this.__thawEntries.some(entry => entry[0] === key && entry[1] === expected);
      }
      set(name, value) {
        const key = String(name);
        const replacement = String(value);
        const first = this.__thawEntries.findIndex(entry => entry[0] === key);
        if (first < 0) this.__thawEntries.push([key, replacement]);
        else {
          this.__thawEntries[first][1] = replacement;
          this.__thawEntries = this.__thawEntries.filter((entry, index) => entry[0] !== key || index === first);
        }
        this.__thawChanged();
      }
      sort() {
        this.__thawEntries = this.__thawEntries.map((entry, index) => [entry, index])
          .sort((left, right) => left[0][0] < right[0][0] ? -1 : left[0][0] > right[0][0] ? 1 : left[1] - right[1])
          .map(item => item[0]);
        this.__thawChanged();
      }
      *entries() { for (let index = 0; index < this.__thawEntries.length; index++) yield this.__thawEntries[index].slice(); }
      *keys() { for (let index = 0; index < this.__thawEntries.length; index++) yield this.__thawEntries[index][0]; }
      *values() { for (let index = 0; index < this.__thawEntries.length; index++) yield this.__thawEntries[index][1]; }
      forEach(callback, thisArg) {
        for (let index = 0; index < this.__thawEntries.length; index++) {
          const entry = this.__thawEntries[index];
          callback.call(thisArg, entry[1], entry[0], this);
        }
      }
      toString() { return this.__thawEntries.map(entry => encodeFormPart(entry[0]) + '=' + encodeFormPart(entry[1])).join('&'); }
      [Symbol.iterator]() { return this.entries(); }
    };
  }
  if (typeof globalThis.URL !== 'function') {
    const toUSV = value => {
      const source = String(value);
      let output = '';
      for (let index = 0; index < source.length; index++) {
        const code = source.charCodeAt(index);
        if (code >= 0xD800 && code <= 0xDBFF && index + 1 < source.length) {
          const next = source.charCodeAt(index + 1);
          if (next >= 0xDC00 && next <= 0xDFFF) {
            output += source.slice(index, index + 2); index++; continue;
          }
        }
        output += code >= 0xD800 && code <= 0xDFFF ? '\uFFFD' : source[index];
      }
      return output;
    };
    const parseURL = (input, base, hasBase) => JSON.parse(__thaw_url_parse(input, base, hasBase));
    const setURL = (href, property, value) => JSON.parse(__thaw_url_set(href, property, value));
    globalThis.URL = class URL {
      constructor(input, base) {
        const value = toUSV(input);
        const hasBase = base !== undefined;
        const baseValue = hasBase ? toUSV(base) : '';
        const parsed = parseURL(value, baseValue, hasBase);
        if (parsed === null) throw new TypeError('Invalid URL');
        this.__thawCommit(parsed);
      }
      __thawCommit(parsed, refresh = true) {
        this.__thawState = parsed;
        if (refresh) this.__thawRefreshParams();
      }
      __thawSet(property, value, refresh = true) {
        const text = toUSV(value);
        const parsed = setURL(this.__thawState.href, property, text);
        if (parsed !== null) this.__thawCommit(parsed, refresh);
      }
      __thawRefreshParams() {
        const parsed = new URLSearchParams(this.__thawState.search);
        const params = this.__thawSearchParams || parsed;
        if (params !== parsed) params.__thawEntries = parsed.__thawEntries;
        params.__thawUpdate = value => this.__thawSet('search', value === '' ? '' : '?' + value, false);
        this.__thawSearchParams = params;
      }
      get protocol() { return this.__thawState.protocol; }
      set protocol(value) { this.__thawSet('protocol', value); }
      get username() { return this.__thawState.username; }
      set username(value) { this.__thawSet('username', value); }
      get password() { return this.__thawState.password; }
      set password(value) { this.__thawSet('password', value); }
      get hostname() { return this.__thawState.hostname; }
      set hostname(value) { this.__thawSet('hostname', value); }
      get port() { return this.__thawState.port; }
      set port(value) { this.__thawSet('port', value); }
      get host() { return this.hostname + (this.port ? ':' + this.port : ''); }
      set host(value) { this.__thawSet('host', value); }
      get pathname() { return this.__thawState.pathname; }
      set pathname(value) { this.__thawSet('pathname', value); }
      get search() { return this.__thawState.search; }
      set search(value) { this.__thawSet('search', value); }
      get searchParams() { return this.__thawSearchParams; }
      get hash() { return this.__thawState.hash; }
      set hash(value) { this.__thawSet('hash', value); }
      get origin() { return this.__thawState.origin; }
      get href() { return this.__thawState.href; }
      set href(value) {
        const parsed = parseURL(toUSV(value), '', false);
        if (parsed === null) throw new TypeError('Invalid URL');
        this.__thawCommit(parsed);
      }
      toString() { return this.href; }
      toJSON() { return this.href; }
      static canParse(input, base) { try { new URL(input, base); return true; } catch (_) { return false; } }
      static parse(input, base) { try { return new URL(input, base); } catch (_) { return null; } }
    };
  }
