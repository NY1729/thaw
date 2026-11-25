// eval var environments: parameter scope and arguments
function t(name, f) { try { console.log(name, f()); } catch (e) { console.log(name, 'THROW', e.constructor.name); } }
t('param-var-arguments-later-param', function () { function f(p = eval('var arguments'), arguments) {} return f(); });
t('param-var-other', function () { function f(p = eval('var q = 5; q')) { return p; } return f(); });
t('arrow-param-var-arguments-named', function () { var f = (p = eval('var arguments'), arguments) => 1; return f(); });
t('body-var-arguments', function () { function f(p = 1) { eval('var arguments = 7'); return arguments; } return f(); });
t('simple-param-var-arguments', function () { function f(p) { eval('var arguments = 7'); return arguments; } return f(); });
