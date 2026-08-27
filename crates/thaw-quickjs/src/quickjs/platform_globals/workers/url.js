  if (typeof globalThis.URLSearchParams !== 'function') {
    const encodeFormPart = value => encodeURIComponent(String(value)).replace(/%20/g, '+');
    const decodeFormPart = value => decodeURIComponent(String(value).replace(/\+/g, ' '));
    globalThis.URLSearchParams = class URLSearchParams {
      constructor(init = '') {
        this.__thawEntries = [];
        this.__thawUpdate = null;
        if (typeof init === 'string') {
          const source = init.charAt(0) === '?' ? init.substring(1) : init;
          if (source !== '') for (const field of source.split('&')) {
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
      entries() { return this.__thawEntries.map(entry => entry.slice())[Symbol.iterator](); }
      keys() { return this.__thawEntries.map(entry => entry[0])[Symbol.iterator](); }
      values() { return this.__thawEntries.map(entry => entry[1])[Symbol.iterator](); }
      forEach(callback, thisArg) {
        for (const entry of this.__thawEntries.slice()) callback.call(thisArg, entry[1], entry[0], this);
      }
      toString() { return this.__thawEntries.map(entry => encodeFormPart(entry[0]) + '=' + encodeFormPart(entry[1])).join('&'); }
      [Symbol.iterator]() { return this.entries(); }
    };
  }
  if (typeof globalThis.URL !== 'function') {
    const normalizePath = path => {
      const absolute = path.charAt(0) === '/';
      const trailing = path.endsWith('/') || path.endsWith('/.') || path.endsWith('/..');
      const output = [];
      for (const part of path.split('/')) {
        if (part === '' || part === '.') continue;
        if (part === '..') output.pop(); else output.push(part);
      }
      let result = (absolute ? '/' : '') + output.join('/');
      if (trailing && result !== '/') result += '/';
      return result || (absolute ? '/' : '');
    };
    const parseAuthority = authority => {
      let userinfo = '', host = authority;
      const at = authority.lastIndexOf('@');
      if (at >= 0) { userinfo = authority.substring(0, at); host = authority.substring(at + 1); }
      let username = '', password = '';
      const colon = userinfo.indexOf(':');
      if (colon < 0) username = userinfo;
      else { username = userinfo.substring(0, colon); password = userinfo.substring(colon + 1); }
      let hostname = host, port = '';
      if (host.charAt(0) === '[') {
        const close = host.indexOf(']');
        if (close < 0) throw new TypeError('Invalid URL');
        hostname = host.substring(0, close + 1);
        if (host.charAt(close + 1) === ':') port = host.substring(close + 2);
      } else {
        const hostColon = host.lastIndexOf(':');
        if (hostColon >= 0) { hostname = host.substring(0, hostColon); port = host.substring(hostColon + 1); }
      }
      if (port !== '' && !/^\d+$/.test(port)) throw new TypeError('Invalid URL');
      return { username, password, hostname: hostname.toLowerCase(), port };
    };
    globalThis.URL = class URL {
      constructor(input, base) {
        const value = String(input);
        const absolute = value.match(/^([A-Za-z][A-Za-z\d+.-]*:)(?:\/\/([^\/?#]*))?([^?#]*)(\?[^#]*)?(#.*)?$/);
        if (absolute) {
          this.__thawProtocol = absolute[1].toLowerCase();
          const authority = absolute[2] === undefined ? '' : absolute[2];
          const parsed = parseAuthority(authority);
          this.__thawUsername = parsed.username;
          this.__thawPassword = parsed.password;
          this.__thawHostname = parsed.hostname;
          this.__thawPort = parsed.port;
          this.__thawPathname = normalizePath(absolute[3] || (authority !== '' ? '/' : ''));
          this.__thawSearch = absolute[4] || '';
          this.__thawHash = absolute[5] || '';
        } else {
          if (base === undefined) throw new TypeError('Invalid URL');
          const parent = base instanceof URL ? base : new URL(base);
          this.__thawProtocol = parent.protocol;
          this.__thawUsername = parent.username;
          this.__thawPassword = parent.password;
          this.__thawHostname = parent.hostname;
          this.__thawPort = parent.port;
          const match = value.match(/^([^?#]*)(\?[^#]*)?(#.*)?$/);
          const relativePath = match[1];
          if (relativePath === '') this.__thawPathname = parent.pathname;
          else if (relativePath.charAt(0) === '/') this.__thawPathname = normalizePath(relativePath);
          else this.__thawPathname = normalizePath(parent.pathname.substring(0, parent.pathname.lastIndexOf('/') + 1) + relativePath);
          this.__thawSearch = match[2] !== undefined ? match[2] : (relativePath === '' ? parent.search : '');
          this.__thawHash = match[3] || '';
        }
        this.__thawNormalizePort();
        this.__thawRefreshParams();
      }
      __thawNormalizePort() {
        if ((this.__thawProtocol === 'http:' && this.__thawPort === '80')
            || (this.__thawProtocol === 'https:' && this.__thawPort === '443')) this.__thawPort = '';
      }
      __thawRefreshParams() {
        const params = new URLSearchParams(this.__thawSearch);
        params.__thawUpdate = value => { this.__thawSearch = value === '' ? '' : '?' + value; };
        this.__thawSearchParams = params;
      }
      get protocol() { return this.__thawProtocol; }
      set protocol(value) { this.__thawProtocol = String(value).replace(/:$/, '').toLowerCase() + ':'; this.__thawNormalizePort(); }
      get username() { return this.__thawUsername; }
      set username(value) { this.__thawUsername = String(value); }
      get password() { return this.__thawPassword; }
      set password(value) { this.__thawPassword = String(value); }
      get hostname() { return this.__thawHostname; }
      set hostname(value) { this.__thawHostname = String(value).toLowerCase(); }
      get port() { return this.__thawPort; }
      set port(value) { const port = String(value); if (port !== '' && !/^\d+$/.test(port)) return; this.__thawPort = port; this.__thawNormalizePort(); }
      get host() { return this.hostname + (this.port ? ':' + this.port : ''); }
      set host(value) { const parsed = parseAuthority(String(value)); this.__thawHostname = parsed.hostname; this.__thawPort = parsed.port; this.__thawNormalizePort(); }
      get pathname() { return this.__thawPathname; }
      set pathname(value) { this.__thawPathname = normalizePath(String(value).charAt(0) === '/' ? String(value) : '/' + String(value)); }
      get search() { return this.__thawSearch; }
      set search(value) { const search = String(value); this.__thawSearch = search === '' ? '' : (search.charAt(0) === '?' ? search : '?' + search); this.__thawRefreshParams(); }
      get searchParams() { return this.__thawSearchParams; }
      get hash() { return this.__thawHash; }
      set hash(value) { const hash = String(value); this.__thawHash = hash === '' ? '' : (hash.charAt(0) === '#' ? hash : '#' + hash); }
      get origin() { return this.__thawHostname === '' || this.__thawProtocol === 'file:' ? 'null' : this.protocol + '//' + this.host; }
      get href() {
        const credentials = this.username || this.password ? this.username + (this.password ? ':' + this.password : '') + '@' : '';
        const authority = this.hostname !== '' || this.protocol === 'file:' ? '//' + credentials + this.host : '';
        return this.protocol + authority + this.pathname + this.search + this.hash;
      }
      set href(value) { const replacement = new URL(value); Object.assign(this, replacement); this.__thawRefreshParams(); }
      toString() { return this.href; }
      toJSON() { return this.href; }
      static canParse(input, base) { try { new URL(input, base); return true; } catch (_) { return false; } }
      static parse(input, base) { try { return new URL(input, base); } catch (_) { return null; } }
    };
  }
