pub(super) fn source(name: &str) -> Option<&'static str> {
    match name {
        "async_hooks" => Some(
            "var storages = globalThis.__thawAsyncLocalStorages || (globalThis.__thawAsyncLocalStorages = new Set()); var nextAsyncId = globalThis.__thawNextAsyncId || 2; var currentAsyncId = 1; var currentTriggerId = 0; var currentResource = {};\n\
             function restoreAfter(storage, previous, result) { if (result && typeof result.then === 'function') return Promise.resolve(result).finally(function() { storage._store = previous; }); storage._store = previous; return result; }\n\
             function AsyncLocalStorage(options) { if (!(this instanceof AsyncLocalStorage)) return new AsyncLocalStorage(options); this._store = options && options.defaultValue; this.name = options && options.name || ''; this.enabled = true; storages.add(this); }\n\
             AsyncLocalStorage.prototype.disable = function() { this.enabled = false; this._store = undefined; };\n\
             AsyncLocalStorage.prototype.getStore = function() { return this.enabled ? this._store : undefined; };\n\
             AsyncLocalStorage.prototype.enterWith = function(store) { this.enabled = true; this._store = store; storages.add(this); };\n\
             AsyncLocalStorage.prototype.run = function(store, callback) { var args = Array.prototype.slice.call(arguments, 2); var previous = this._store; this.enabled = true; this._store = store; try { return restoreAfter(this, previous, callback.apply(null, args)); } catch (error) { this._store = previous; throw error; } };\n\
             AsyncLocalStorage.prototype.exit = function(callback) { var args = Array.prototype.slice.call(arguments, 1); var previous = this._store; this._store = undefined; try { return restoreAfter(this, previous, callback.apply(null, args)); } catch (error) { this._store = previous; throw error; } };\n\
             function captureStores() { return Array.from(storages).map(function(storage) { return [storage, storage.getStore()]; }); }\n\
             function invokeCaptured(captured, callback, thisArg, args, index) { if (index === captured.length) return callback.apply(thisArg, args); var entry = captured[index]; return entry[0].run(entry[1], function() { return invokeCaptured(captured, callback, thisArg, args, index + 1); }); }\n\
             AsyncLocalStorage.bind = function(callback) { var captured = captureStores(); return function() { return invokeCaptured(captured, callback, this, Array.prototype.slice.call(arguments), 0); }; };\n\
             AsyncLocalStorage.snapshot = function() { var captured = captureStores(); return function(callback) { return invokeCaptured(captured, callback, null, Array.prototype.slice.call(arguments, 1), 0); }; };\n\
             function AsyncResource(type, options) { if (!(this instanceof AsyncResource)) return new AsyncResource(type, options); this.type = String(type); this._asyncId = nextAsyncId++; globalThis.__thawNextAsyncId = nextAsyncId; this._triggerAsyncId = options && options.triggerAsyncId !== undefined ? Number(options.triggerAsyncId) : currentAsyncId; this._destroyed = false; }\n\
             AsyncResource.prototype.asyncId = function() { return this._asyncId; }; AsyncResource.prototype.triggerAsyncId = function() { return this._triggerAsyncId; };\n\
             AsyncResource.prototype.runInAsyncScope = function(callback, thisArg) { var args = Array.prototype.slice.call(arguments, 2); var previousId = currentAsyncId, previousTrigger = currentTriggerId, previousResource = currentResource; currentAsyncId = this._asyncId; currentTriggerId = this._triggerAsyncId; currentResource = this; try { return callback.apply(thisArg, args); } finally { currentAsyncId = previousId; currentTriggerId = previousTrigger; currentResource = previousResource; } };\n\
             AsyncResource.prototype.emitDestroy = function() { this._destroyed = true; return this; }; AsyncResource.prototype.bind = function(callback, thisArg) { var self = this; return function() { return self.runInAsyncScope(callback, thisArg === undefined ? this : thisArg, ...arguments); }; };\n\
             AsyncResource.bind = function(callback, type, thisArg) { return new AsyncResource(type || callback.name || 'bound-anonymous-fn').bind(callback, thisArg); };\n\
             function createHook(callbacks) { return { enable: function() { return this; }, disable: function() { return this; }, callbacks: callbacks || {} }; }\n\
             function executionAsyncId() { return currentAsyncId; } function triggerAsyncId() { return currentTriggerId; } function executionAsyncResource() { return currentResource; }\n\
             module.exports = { AsyncLocalStorage: AsyncLocalStorage, AsyncResource: AsyncResource, createHook: createHook, executionAsyncId: executionAsyncId, triggerAsyncId: triggerAsyncId, executionAsyncResource: executionAsyncResource }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        _ => None,
    }
}
