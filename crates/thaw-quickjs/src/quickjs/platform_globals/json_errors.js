(() => {
  // QuickJS reports a cycle as just "circular reference"; V8 names the whole circle.
  const nativeStringify = JSON.stringify;
  const ctorName = value => {
    const ctor = value.constructor;
    return typeof ctor === 'function' && ctor.name ? ctor.name : 'Object';
  };
  const circleMessage = root => {
    const path = [];
    const done = new Set();
    let found = null;
    const visit = (value, key) => {
      if (found || done.has(value)) return;
      const at = path.findIndex(entry => entry.value === value);
      if (at >= 0) {
        const links = path.slice(at);
        let text = "Converting circular structure to JSON\n    --> starting at object with constructor '" + links[0].ctor + "'";
        const prefixEnd = Math.min(links.length, 3);
        const link = i => "\n    |     " + links[i].key + " -> object with constructor '" + links[i].ctor + "'";
        for (let i = 1; i < prefixEnd; i++) text += link(i);
        if (links.length > prefixEnd + 1) text += "\n    |     ...";
        for (let i = Math.max(prefixEnd, links.length - 1); i < links.length; i++) text += link(i);
        found = text + "\n    --- " + key + " closes the circle";
        return;
      }
      path.push({ value, ctor: ctorName(value), key });
      const array = Array.isArray(value);
      for (const name of Object.keys(value)) {
        const child = value[name];
        if (child !== null && typeof child === 'object') {
          visit(child, array ? 'index ' + name : "property '" + name + "'");
        }
      }
      path.pop();
      done.add(value);
    };
    if (root !== null && typeof root === 'object') visit(root, '');
    return found;
  };
  const stringify = function stringify(value, replacer, space) {
    try {
      return nativeStringify.apply(this, arguments);
    } catch (error) {
      if (error && typeof error.message === 'string' && error.message.endsWith('circular reference')) {
        const message = circleMessage(value);
        if (message) throw new TypeError(message);
      }
      throw error;
    }
  };
  Object.defineProperty(stringify, 'length', { value: 3, configurable: true });
  Object.defineProperty(JSON, 'stringify', { value: stringify, writable: true, configurable: true });
})();
