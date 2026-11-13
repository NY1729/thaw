// Node-shaped descriptors and prototype of navigator. Locale and core count are not printed.
console.log(typeof navigator, typeof Navigator);
console.log(Object.getOwnPropertyNames(Navigator).join(','));
console.log(Object.getOwnPropertyNames(Navigator.prototype).join(','));
console.log(Navigator.length, Navigator.name, typeof Navigator.prototype);
const pd = Object.getOwnPropertyDescriptor(Navigator.prototype, 'platform');
console.log(pd.enumerable, pd.configurable, pd.set, typeof pd.get, pd.get.name);
console.log(Object.getOwnPropertyDescriptor(Navigator.prototype, 'constructor').enumerable);
console.log(Object.getOwnPropertyDescriptor(Navigator.prototype, Symbol.toStringTag));
console.log(navigator);
console.log(Object.prototype.toString.call(navigator));
console.log(Object.keys(navigator).length, Object.getOwnPropertyNames(navigator).length);
console.log(Object.getPrototypeOf(navigator) === Navigator.prototype, navigator instanceof Navigator);
console.log(typeof navigator.hardwareConcurrency, navigator.hardwareConcurrency >= 1);
console.log(typeof navigator.language, navigator.language === navigator.languages[0]);
console.log(Array.isArray(navigator.languages), navigator.languages.length, Object.isFrozen(navigator.languages));
console.log(navigator.languages === navigator.languages, navigator.languages[0] === navigator.language);
console.log(navigator.userAgent, navigator.platform.startsWith("Linux"));
