// Generate a self-checking browser fixture. No live services or credentials are used.
// node tools/test-explorer-interaction.cjs; chromium --headless --dump-dom file:///tmp/boreal-explorer-test.html
const fs = require('fs');
const path = require('path');
const partial = fs.readFileSync(path.join(__dirname, '../tmpl/html/partials/explorer-interaction.html'), 'utf8');
let pager = fs.readFileSync(path.join(__dirname, '../tmpl/html/partials/explorer-pagination.html'), 'utf8')
    .replace(/{%[^%]*%}/g, '').replace(/{{ pagination.page }}/g, '1').replace(/{{ pagination.size }}/g, '50')
    .replace(/{{ pagination.total }}/g, '60').replace(/{{ pagination.pages }}/g, '2');
const html = `<!doctype html><html><body><main>${pager}
<form id="filter" method="get"><input name="q"><button id="export" formaction="/keeper/export.xlsx">Export</button><button id="print" formtarget="_blank">Print</button></form>
<form data-explorer-tags action="/my-drive/tags" method="post">
<input name="selected_item_ids" type="hidden"><select name="tag"><option value="review" data-color="#ffdd00">Review</option></select>
<button id="apply">Apply</button><button id="remove" formaction="/my-drive/tags/remove">Remove</button>
<table><tbody>${['one','two'].map(id => `<tr><td><input class="my-drive-item-select" type="checkbox" value="${id}"></td><td><span data-explorer-tags-cell></span></td><td><div class="boreal-permissions-scroll" tabindex="0">${'<div>Permission identity</div>'.repeat(30)}</div></td></tr>`).join('')}</tbody></table>
</form></main><div style="height:2000px"></div>
<script>
localStorage.removeItem('boreal.explorer.pageSize');
const posts = [], completions = [];
const form = document.querySelector('[data-explorer-tags]');
form.addEventListener('submit', () => { form.elements.selected_item_ids.value = [...document.querySelectorAll('.my-drive-item-select:checked')].map(b => b.value).join(','); });
window.fetch = (url, options = {}) => {
    if (options.method === 'POST') {
        posts.push(new URLSearchParams(options.body));
        return new Promise((resolve, reject) => completions.push({resolve, reject}));
    }
    return Promise.resolve(new Response('<table><tr><td><input class="my-drive-item-select" value="one"></td><td><span data-explorer-tags-cell><span>Review</span></span></td></tr><tr><td><input class="my-drive-item-select" value="two"></td><td><span data-explorer-tags-cell></span></td></tr></table>'));
};
</script>${partial}
<script>
(async () => {
 const assert = (condition, message) => { if (!condition) throw new Error(message); };
 const settle = () => new Promise(resolve => setTimeout(resolve, 0));
 const boxes = [...document.querySelectorAll('.my-drive-item-select')];
 const select = (one, two) => { boxes[0].checked=one; boxes[1].checked=two; boxes[0].dispatchEvent(new Event('change',{bubbles:true})); };
 const submit = id => form.requestSubmit(document.getElementById(id));
 const status = document.getElementById('explorer-save-status');
 const scroll = document.querySelector('.boreal-permissions-scroll');
 assert(scroll.clientHeight <= 200 && scroll.scrollHeight > scroll.clientHeight, 'permission entries must scroll within 200px');
 assert(document.querySelector('[data-page-step="-1"]').disabled, 'first page has no Previous');
 assert(!document.querySelector('[data-page-step="1"]').disabled, 'Next is enabled');
 assert(!document.getElementById('explorer-select-matching').hidden, 'all-page selection is available before selecting rows');
 document.getElementById('explorer-select-matching').click(); submit('apply');
 assert(posts.length === 1 && posts[0].get('selected_item_ids') === 'one,two' && posts[0].get('all_matching') === 'true', 'first operation captures first selection');
 assert(document.querySelector('.boreal-pending-tag')?.textContent === 'Review', 'optimistic tag appears immediately');
 select(false,true); submit('remove');
 assert(posts.length === 1, 'second write waits for first');
 const filterEvent = new Event('submit',{bubbles:true,cancelable:true}); document.getElementById('filter').dispatchEvent(filterEvent);
 assert(filterEvent.defaultPrevented, 'filter navigation blocked during saves');
 completions.shift().resolve(new Response(null,{status:204})); await settle();
 assert(posts.length === 2 && posts[1].get('selected_item_ids') === 'two', 'queued operation keeps its own IDs');
 completions.shift().resolve(new Response(null,{status:204})); await settle(); await settle();
 assert(status.textContent.startsWith('Tags saved'), 'successful writes reported');
 assert(boxes[1].checked && !boxes[0].checked, 'selection survives reconciliation');
 document.getElementById('explorer-select-page').click();
 assert(document.getElementById('explorer-select-matching').hidden, 'stale filters require refresh before another all-page selection');
 submit('apply');
 assert(posts[2].get('all_matching') === 'false', 'later action uses explicit page selection');
 completions.shift().reject(new Error('simulated network failure')); await settle();
 assert(!document.getElementById('explorer-retry').hidden && status.textContent.includes('Not saved'), 'save error is visible and retryable');
 document.getElementById('explorer-retry').click();
 assert(posts[3].get('operation_id') === posts[2].get('operation_id'), 'retry preserves operation identity');
 completions.shift().resolve(new Response(null,{status:204})); await settle(); await settle();
 assert(status.textContent.startsWith('Tags saved'), 'retry completes');
 document.getElementById('explorer-clear-selection').click();
 assert(boxes.every(box => !box.checked), 'clear selection clears page');
 for (const id of ['export','print']) {
     document.getElementById('filter').dispatchEvent(new SubmitEvent('submit',{bubbles:true,cancelable:true,submitter:document.getElementById(id)}));
     assert(!document.querySelector('main').inert, 'export and print do not lock the page');
 }

 document.body.dataset.test = 'PASS';
 const result=document.createElement('pre');result.id='test-result';result.textContent='PASS: paging controls, permission scrolling, optimistic tags, ordered writes, selection snapshots, navigation guard, retry identity and reconciliation';document.body.append(result);
})().catch(error => { document.body.dataset.test='FAIL';const result=document.createElement('pre');result.id='test-result';result.textContent=error.stack;document.body.append(result); });
</script></body></html>`;
fs.writeFileSync('/tmp/boreal-explorer-test.html', html);
console.log('/tmp/boreal-explorer-test.html');
