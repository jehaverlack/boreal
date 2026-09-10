// Run with node tools/test-web-controls.cjs. No browser or live BOREAL required.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const base = fs.readFileSync(path.join(__dirname, '../tmpl/html/base.html'), 'utf8');
const start = base.indexOf('            const quitButton =');
const end = base.indexOf('        })();', start);
const shutdownScript = base.slice(start, end);

function shutdown({ platform = 'Linux', close = 'blocked', response = {ok: true, status: 200} } = {}) {
    const timers = [];
    const state = { attempts: 0, popups: 0, message: '', quit: null, heartbeat: null };
    const elements = {
        'boreal-quit': { addEventListener: (_, fn) => state.quit = fn },
        'borealStoppedModal': {},
        'boreal-close-tab-blocked': { classList: { remove: () => {} } },
        'boreal-close-tab-blocked-message': { set textContent(value) { state.message = value; } },
    };
    const window = {
        closed: false,
        close() {
            state.attempts++;
            if (close === 'throws') throw new Error('blocked');
            if (close === 'allowed') this.closed = true;
        },
        setTimeout(fn) { timers.push(fn); },
        setInterval(fn) { state.heartbeat = fn; },
        confirm() { return false; },
    };
    vm.runInNewContext(shutdownScript, {
        document: { getElementById: id => elements[id] }, window,
        navigator: { platform },
        bootstrap: { Modal: class { show() { state.popups++; } } },
        fetch: async () => response,
    });
    state.flush = () => { while (timers.length) timers.shift()(); };
    return state;
}

(async () => {
    for (const close of ['blocked', 'throws', 'allowed']) {
        const state = shutdown({close});
        await state.quit();
        state.flush();
        assert.equal(state.attempts, 1);
        assert.equal(state.popups, close === 'allowed' ? 0 : 1);
        if (close !== 'allowed') assert.match(state.message, /Ctrl\+W/);
        await state.heartbeat();
        state.flush();
        assert.equal(state.attempts, 1, 'do not repeat close attempts');
    }
    const mac = shutdown({platform: 'MacIntel'});
    await mac.quit(); mac.flush();
    assert.match(mac.message, /Command\+W/);
    const stopped = shutdown({response: {ok: false}});
    await stopped.heartbeat(); stopped.flush();
    assert.equal(stopped.attempts, 0, 'one missed heartbeat is tolerated');
    await stopped.heartbeat(); stopped.flush();
    assert.equal(stopped.attempts, 1, 'external shutdown also attempts closure');
    assert.equal(stopped.popups, 1);
    const canceled = shutdown({response: {status: 409, text: async () => 'Busy'}});
    await canceled.quit(); canceled.flush();
    assert.equal(canceled.attempts, 0, 'canceling shutdown must keep the tab');
    assert(!base.includes('id="boreal-close-tab"'), 'obsolete close button removed');
    console.log('Shutdown controls: 6 scenarios passed');
})().catch(error => { console.error(error); process.exitCode = 1; });

const keeper = fs.readFileSync(path.join(__dirname, '../tmpl/html/keeper.html'), 'utf8');
const keeperScript = keeper.slice(keeper.indexOf('<script>') + 8, keeper.indexOf('</script>'));
function element(extra = {}) {
    return { listeners: {}, addEventListener(event, fn) { this.listeners[event] = fn; }, ...extra };
}
const boxes = [element({value: 'folder-a', dataset: {kind: 'folder'}, checked: false}),
    element({value: 'record-b', dataset: {kind: 'record'}, checked: false}),
    element({value: 'record-b', dataset: {kind: 'record'}, checked: false})];
const tagForm = element({elements: {selected_folder_uids: {}, selected_record_uids: {}}});
const controls = {
    'keeper-filter-form': {elements: {sort: {value: ''}, direction: {value: ''}, tag: {value: ''}}},
    'keeper-tag-form': tagForm,
    'keeper-select-all': element({checked: false}),
    'keeper-selected-count': {}, 'keeper-apply': {disabled: true}, 'keeper-remove': {disabled: true},
};
vm.runInNewContext(keeperScript, {document: {
    getElementById: id => controls[id],
    querySelectorAll: selector => selector === '.keeper-select' ? boxes : selector === '.keeper-select:checked' ? boxes.filter(box => box.checked) : [],
}, alert: () => {} });
controls['keeper-select-all'].checked = true;
controls['keeper-select-all'].listeners.change({target: controls['keeper-select-all']});
assert.equal(controls['keeper-selected-count'].textContent, '2 selected');
assert.equal(controls['keeper-apply'].disabled, false);
tagForm.listeners.submit({currentTarget: tagForm, preventDefault: () => assert.fail('selected items should submit')});
assert.equal(tagForm.elements.selected_folder_uids.value, 'folder-a');
assert.equal(tagForm.elements.selected_record_uids.value, 'record-b');
controls['keeper-select-all'].checked = false;
controls['keeper-select-all'].listeners.change({target: controls['keeper-select-all']});
assert.equal(controls['keeper-selected-count'].textContent, '0 selected');
assert.equal(controls['keeper-apply'].disabled, true);
assert.equal(controls['keeper-remove'].disabled, true);
console.log('Keeper tag controls: selection, deduplication and submission passed');
