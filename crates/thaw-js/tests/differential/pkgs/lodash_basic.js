// pkg: lodash
const _ = require('lodash');
const log = (...a) => console.log(...a.map((x) => JSON.stringify(x)));
log(_.chunk([1, 2, 3, 4, 5], 2), _.uniq([3, 1, 3, 2, 1]), _.sum([1, 2, 3]), _.camelCase('Foo Bar-baz'), _.get({ a: [{ b: 5 }] }, 'a[0].b'), _.groupBy([6.1, 4.2, 6.3], Math.floor));
log(_.merge({ a: 1, b: { c: 2 } }, { b: { d: 3 }, e: 4 }), _.pick({ a: 1, b: 2, c: 3 }, ['a', 'c']), _.omit({ a: 1, b: 2, c: 3 }, 'b'), _.keyBy([{ id: 'x', n: 1 }, { id: 'y', n: 2 }], 'id'));
log(_.sortBy([{ n: 'b', a: 2 }, { n: 'a', a: 2 }, { n: 'c', a: 1 }], ['a', 'n']), _.orderBy([3, 1, 2], [], ['desc']), _.flatten([1, [2, [3, [4]]]]), _.flattenDeep([1, [2, [3, [4]]]]));
log(_.range(5), _.range(1, 10, 3), _.times(3, (i) => i * i), _.zip(['a', 'b'], [1, 2]), _.fromPairs([['a', 1], ['b', 2]]), _.toPairs({ a: 1 }), _.invert({ a: 'x', b: 'y' }));
log(_.debounce.length, _.isEqual({ a: [1, { b: 2 }] }, { a: [1, { b: 2 }] }), _.isEmpty({}), _.isEmpty([1]), _.cloneDeep({ a: [1, { b: 2 }] }), _.isString('x'), _.isNumber(NaN), _.isPlainObject([]));
log(_.capitalize('fOO'), _.kebabCase('fooBar baz'), _.snakeCase('Foo Bar'), _.startCase('foo_bar'), _.pad('abc', 7, '*'), _.truncate('hello world foo', { length: 11 }), _.escape('<a href="x">&</a>'));
log(_.max([4, 9, 2]), _.min([4, 9, 2]), _.mean([1, 2, 3, 4]), _.maxBy([{ n: 1 }, { n: 5 }], 'n'), _.countBy(['one', 'two', 'three'], 'length'), _.partition([1, 2, 3, 4], (n) => n % 2), _.difference([1, 2, 3], [2]), _.intersection([1, 2], [2, 3]), _.union([1], [1, 2]), _.without([1, 2, 1], 1));
log(_.map({ a: 1, b: 2 }, (v, k) => k + v), _.filter([1, 2, 3, 4], (n) => n > 2), _.reduce([1, 2, 3], (s, n) => s + n, 10), _.find([{ a: 1 }, { a: 2 }], { a: 2 }), _.some([1, 2], (n) => n > 1), _.every([1, 2], (n) => n > 1), _.includes([1, 2, 3], 2), _.reject([1, 2, 3], (n) => n % 2));
log(_.set({}, 'a.b.c', 1), _.has({ a: { b: 1 } }, 'a.b'), _.result({ a: () => 5 }, 'a'), _.defaults({ a: 1 }, { a: 2, b: 3 }), _.assign({ a: 1 }, { b: 2 }), _.mapValues({ a: 1, b: 2 }, (n) => n * 2), _.mapKeys({ a: 1 }, (v, k) => k + k), _.uniqBy([1.2, 1.5, 2.1], Math.floor));
log(_.take([1, 2, 3], 2), _.drop([1, 2, 3], 2), _.takeWhile([1, 2, 3, 1], (n) => n < 3), _.head([5, 6]), _.last([5, 6]), _.tail([5, 6, 7]), _.compact([0, 1, false, 2, '', 3]), _.reverse([1, 2, 3]), _.concat([1], 2, [3]), _.indexOf([1, 2, 1], 1, 1), _.nth([1, 2, 3], -1));
log(_.padStart('5', 3, '0'), _.repeat('ab', 3), _.words('fred, barney, & pebbles'), _.trim('  x  '), _.upperFirst('foo'), _.lowerCase('--Foo-Bar--'), _.deburr('déjà vu'), _.split('a-b-c', '-', 2), _.replace('abc', 'b', 'X'), _.startsWith('abc', 'b', 1));
