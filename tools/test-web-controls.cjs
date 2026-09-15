// Run with node tools/test-web-controls.cjs. No browser or live BOREAL required.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const base = fs.readFileSync(path.join(__dirname, '../tmpl/html/base.html'), 'utf8');
const tagWindow = {};
const tagScriptStart = base.indexOf('        window.borealToggleTag =');
vm.runInNewContext(base.slice(tagScriptStart, base.indexOf('</script>', tagScriptStart)), {window: tagWindow});
const toggleTag = tagWindow.borealToggleTag;
assert.equal(toggleTag('', 'keep'), 'keep');
assert.equal(toggleTag('keep', 'review'), 'keep,review');
assert.equal(toggleTag('keep,review', 'review'), 'keep,!review');
assert.equal(toggleTag('keep,!review', 'review'), 'keep');
assert.equal(toggleTag('keep,!review', ''), '');
assert.equal(toggleTag('', ''), '');
assert.equal(toggleTag('keeper', 'keep'), 'keeper,keep');
assert.equal(toggleTag('keep,!review', 'keep'), '!review,!keep');
console.log('Shared tag cycle: include, exclude, clear and multiple predicates passed');
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
const vaultPills = [element({dataset: {keeperTag: 'keep'}}), element({dataset: {keeperTag: 'review'}}), element({dataset: {keeperTag: ''}})];
const userPills = [element({dataset: {keeperUserTag: 'needs-review'}}), element({dataset: {keeperUserTag: ''}})];
let filterSubmissions = 0;
const tagForm = element({elements: {selected_folder_uids: {}, selected_record_uids: {}}});
const controls = {
    'keeper-filter-form': {requestSubmit() { filterSubmissions++; }, elements: {sort: {value: ''}, direction: {value: ''}, tag: {value: ''}, user_tag: {value: ''}}},
    'keeper-tag-form': tagForm,
    'keeper-select-all': element({checked: false}),
    'keeper-selected-count': {}, 'keeper-apply': {disabled: true}, 'keeper-remove': {disabled: true},
};
vm.runInNewContext(keeperScript, {document: {
    getElementById: id => controls[id],
    querySelectorAll: selector => selector === '[data-keeper-tag]' ? vaultPills : selector === '[data-keeper-user-tag]' ? userPills : selector === '.keeper-select' ? boxes : selector === '.keeper-select:checked' ? boxes.filter(box => box.checked) : [],
}, window: tagWindow, alert: () => {} });
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

userPills[0].listeners.click({preventDefault() {}});
assert.equal(controls['keeper-filter-form'].elements.user_tag.value, 'needs-review');
userPills[0].listeners.contextmenu({preventDefault() {}});
assert.equal(controls['keeper-filter-form'].elements.user_tag.value, '!needs-review');
userPills[1].listeners.click({preventDefault() {}});
assert.equal(controls['keeper-filter-form'].elements.user_tag.value, '');
assert.equal(filterSubmissions, 3);
userPills[0].listeners.click({preventDefault() {}});
userPills[0].listeners.click({preventDefault() {}});
assert.equal(controls['keeper-filter-form'].elements.user_tag.value, '!needs-review');
userPills[0].listeners.click({preventDefault() {}});
assert.equal(controls['keeper-filter-form'].elements.user_tag.value, '');
controls['keeper-filter-form'].elements.user_tag.value = 'keep';
userPills[0].listeners.click({preventDefault() {}});
assert.equal(controls['keeper-filter-form'].elements.user_tag.value, 'keep,needs-review');
userPills[0].listeners.click({preventDefault() {}});
assert.equal(controls['keeper-filter-form'].elements.user_tag.value, 'keep,!needs-review');
console.log('Keeper user-tag controls: three-click cycle, combined terms and clear passed');

const metadataScript = base.slice(base.indexOf('            const metadataModalElement ='), start);
function metadataDialog({background = false, initial = 'running', selected = []} = {}) {
    const events = {};
    const state = {status: initial, requests: 0, shows: 0, reloads: 0, poll: null};
    const content = {
        innerHTML: 'idle',
        querySelector(selector) {
            if (selector !== '#metadata-update-modal-content' || this.innerHTML.includes('Metadata update complete')) return null;
            return {dataset: {updating: String(this.innerHTML === 'running'), complete: String(this.innerHTML === 'idle')}};
        },
    };
    state.choices = [{name:'keeper',disabled:false,checked:false},{name:'my_drive',disabled:true,checked:false}];
    const modal = {
        querySelectorAll() { return state.choices; },
        querySelector(selector) { return selector === '.modal-content' ? content : content.querySelector(selector); },
        addEventListener(event, callback) { events[event] = callback; },
    };
    const context = vm.createContext({
        document: {
            getElementById: () => modal,
            querySelector: () => null,
            createElement: () => ({...content}),
        },
        window: {
            localStorage: {getItem: () => JSON.stringify(selected), setItem() {}},
            sessionStorage: {getItem: () => String(background), removeItem() {}, setItem() {}},
            location: {reload() { state.reloads++; }},
            setInterval(callback) { state.poll = callback; },
        },
        bootstrap: {Modal: {getOrCreateInstance: () => ({show() { state.shows++; events['show.bs.modal'](); }})}},
        fetch: async url => { state.requests++; if (url === '/metadata/acknowledge-error') state.status = 'idle'; return {ok: true, text: async () => state.status}; },
        refreshDataAges() {}, refreshMetadataStatusAge() {},
    });
    vm.runInContext(metadataScript, context);
    state.refresh = () => vm.runInContext('refreshMetadataModal()', context);
    state.dismiss = () => { events['hide.bs.modal'](); events['hidden.bs.modal'](); };
    state.acknowledge = () => events.submit({target:{id:'metadata-error-acknowledge',querySelector:()=>({disabled:false})},preventDefault(){}});
    state.content = content;
    return state;
}
(async () => {
    const settle = () => new Promise(resolve => setImmediate(resolve));
    for (const background of [false, true]) {
        const state = metadataDialog({background});
        await settle();
        assert.equal(state.shows, background ? 0 : 1);
        state.status = 'idle';
        await Promise.all([state.refresh(), state.refresh()]);
        assert.equal(state.requests, 2, 'overlapping polls must be coalesced');
        assert.match(state.content.innerHTML, /Metadata update complete/);
        assert.match(state.content.innerHTML, />Done</);
        assert.equal(state.shows, background ? 1 : 2, 'completion is shown even in background');
        state.poll(); await state.refresh();
        assert.equal(state.requests, 2, 'completion must remain until acknowledged');
        assert.equal(state.reloads, 0);
        state.dismiss();
        assert.equal(state.reloads, 1, 'acknowledgement refreshes viewer data');
        await state.refresh();
        assert.equal(state.content.innerHTML, 'idle', 'next opening permits a new update');
    }
    const idle = metadataDialog({initial: 'idle'});
    await settle();
    assert.equal(idle.shows, 0, 'idle page loads must not announce completion');
    const resumed = metadataDialog({initial: 'idle', background: true});
    await settle();
    assert.match(resumed.content.innerHTML, /Metadata update complete/, 'background completion survives navigation');
    const failed = metadataDialog({background: true});
    await settle(); failed.status = 'failed'; await failed.refresh();
    assert.equal(failed.content.innerHTML, 'failed');
    assert.equal(failed.shows, 1, 'background failures must be visible');
    await failed.acknowledge();
    assert.equal(failed.content.innerHTML,'idle','acknowledgement returns to source selection');
    assert(!failed.content.innerHTML.includes('Metadata update complete'),'error acknowledgement is not a successful update');
    const remembered = metadataDialog({initial:'idle',selected:['keeper','my_drive']}); await settle();
    assert.equal(remembered.choices[0].checked,true);
    assert.equal(remembered.choices[1].checked,false,'unavailable sources are not restored');
    console.log('Metadata dialog: completion, acknowledgement, background updates, failures and polling passed');
})().catch(error => { console.error(error); process.exitCode = 1; });

for (const pill of [vaultPills[0], vaultPills[1], vaultPills[1]]) pill.listeners.click({preventDefault() {}});
assert.equal(controls['keeper-filter-form'].elements.tag.value, 'keep,!review');
vaultPills[1].listeners.click({preventDefault() {}});
assert.equal(controls['keeper-filter-form'].elements.tag.value, 'keep');
vaultPills[2].listeners.click({preventDefault() {}});
assert.equal(controls['keeper-filter-form'].elements.tag.value, '');
assert.equal(controls['keeper-filter-form'].elements.user_tag.value, 'keep,!needs-review', 'Any Folder preserves user filters');
console.log('Keeper vault tags: mixed filters, three-click cycle and Any Folder passed');

const guide = fs.readFileSync(path.join(__dirname, '../tmpl/html/partials/google-setup-guide.html'), 'utf8');
const guideSteps = [{},{},{},{},{}], guideBack = element(), guideNext = element(), guideCount = {};
vm.runInNewContext(guide.match(/<script>([\s\S]*?)<\/script>/)[1], {document: {
    querySelectorAll: () => guideSteps,
    getElementById: id => ({'google-setup-back':guideBack,'google-setup-next':guideNext,'google-setup-count':guideCount}[id]),
}});
guideNext.listeners.click();
assert.equal(guideSteps[1].hidden,false);
assert.equal(guideSteps[0].hidden,true);
guideNext.listeners.click(); guideNext.listeners.click();
assert.equal(guideCount.textContent,'Step 4 of 5');
assert.equal(guideNext.textContent,'Proceed to Step 5');
guideNext.listeners.click();
assert.equal(guideNext.disabled,true);
assert.equal(guideNext.hidden,true);
assert.equal(guideCount.textContent,'Step 5 of 5');
guideBack.listeners.click();
assert.equal(guideNext.disabled,false);
assert.equal(guideSteps[3].hidden,false);
assert.equal(guideNext.hidden,false);
console.log('Google setup guide: next, back and final step passed');
