// The native addon proxy layer (plan tj107 sections 2 and 4, PR 6b-1): the export listing, the owner
// lookup and the factory shells. napi_proxy.rs runs this file as a script; its completion value is the
// factory below, called once per realm with `cap`, the private natives (exports, owner, invoker).
// The proxy and function factories are stubs until PR 6b-2 and 6b-3.
(function (cap) {
  'use strict';
  // ponytail: no primordial capture; this runs at realm build, before user code can replace the methods used here.
  const exports = () => JSON.parse(cap.exports());

  // RJ runtime.js each_export: qualified packages list only their own prefixed names.
  const eachExport = (info, prefix, packageName, install) => {
    const qualified = info.qualifiedPackages.indexOf(packageName) !== -1;
    info.names.forEach(name => {
      if (qualified) {
        if (!name.startsWith(prefix)) return;
        install(name, name.slice(prefix.length));
      } else if (name.indexOf('::') === -1) {
        install(name, name);
      }
    });
  };

  const ownerOf = handle => cap.owner(handle);

  const notImplemented = name => () => {
    throw new TypeError(`Native addon ${name} is not implemented`);
  };

  return {
    exports,
    eachExport,
    ownerOf,
    proxyForHandle: notImplemented('proxies'),
    functionForHandle: notImplemented('functions'),
  };
});
