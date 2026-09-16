// Generate a self-checking browser fixture. No live services or credentials are used.
// node tools/test-explorer-interaction.cjs; chromium --headless --dump-dom file:///tmp/boreal-explorer-test.html
const fs = require('fs');
const path = require('path');
const partial = fs.readFileSync(path.join(__dirname, '../tmpl/html/partials/explorer-interaction.html'), 'utf8');
const driveTemplate = fs.readFileSync(path.join(__dirname, '../tmpl/html/my-drive.html'), 'utf8');
const driveScript = driveTemplate.slice(driveTemplate.lastIndexOf('<script>'), driveTemplate.lastIndexOf('</script>') + 9);
const columnWidths = fs.readFileSync(path.join(__dirname, '../tmpl/html/partials/column-widths.html'), 'utf8');
const headerSelection = fs.readFileSync(path.join(__dirname, '../tmpl/html/partials/explorer-selection.html'), 'utf8');
const migrateButton = driveTemplate.match(/<button id="my-drive-migrate-button"[^>]*>[\s\S]*?<\/button>/)[0];
let pager = fs.readFileSync(path.join(__dirname, '../tmpl/html/partials/explorer-pagination.html'), 'utf8')
    .replace(/{%[^%]*%}/g, '').replace(/{{ pagination.page }}/g, '1').replace(/{{ pagination.size }}/g, '50')
    .replace(/{{ pagination.total }}/g, '60').replace(/{{ pagination.pages }}/g, '2');
const html = `<!doctype html><html><head><style>body {font:16px/1.5 sans-serif} th,td {padding:.5rem} .small {font-size:.875em} .form-check-input {width:1em;height:1em}</style></head><body><main>${pager}
<form id="filter" method="get"><input name="q"><button id="export" formaction="/keeper/export.xlsx">Export</button><button id="print" formtarget="_blank">Print</button></form>
<form data-explorer-tags action="/my-drive/tags" method="post">
<input id="my-drive-selected-item-ids" name="selected_item_ids" type="hidden"><select name="tag" required><option value="review" data-color="#ffdd00">Review</option></select>
<input name="tag_filter" value="filtered-tag"><input name="owner_identity_tag_filter" value="owner-tag">${migrateButton}<button id="my-drive-update-metadata-button">Metadata</button><button id="my-drive-apply-tag-button">Apply</button><button id="my-drive-remove-tag-button" formaction="/my-drive/tags/remove">Remove</button><span id="my-drive-selection-count"></span>${["heading","items","files","folders","size","permissions"].map(id=>`<span id="explorer-summary-${id}"></span>`).join('')}
<table data-column-widths="my-drive"><thead><tr><th><input type="checkbox" id="my-drive-select-all">${headerSelection}</th><th>Tags</th><th>Permissions</th></tr></thead><tbody>${['one','two'].map(id => `<tr data-size-bytes="10" data-is-directory="false" data-permission-count="1"><td><input class="my-drive-item-select" type="checkbox" value="${id}"></td><td><span data-explorer-tags-cell></span></td><td><div class="boreal-permissions-scroll" tabindex="0">${'<div>Permission identity</div>'.repeat(30)}</div></td></tr>`).join('')}</tbody></table>
</form></main><div style="height:2000px"></div>
<script>
localStorage.removeItem('boreal.explorer.pageSize');
localStorage.setItem('boreal.columnWidths.v1:' + location.pathname + ':my-drive:', JSON.stringify([36,200,200]));
const posts = [], completions = [];
const form = document.querySelector('[data-explorer-tags]');

window.fetch = (url, options = {}) => {
    if (options.method === 'POST') {
        posts.push(new URLSearchParams(options.body));
        return new Promise((resolve, reject) => completions.push({resolve, reject}));
    }
    return Promise.resolve(new Response('<table><tr data-size-bytes="10" data-is-directory="false" data-permission-count="1"><td><input class="my-drive-item-select" value="one"></td><td><span data-explorer-tags-cell><span>Review</span></span></td></tr><tr data-size-bytes="10" data-is-directory="false" data-permission-count="1"><td><input class="my-drive-item-select" value="two"></td><td><span data-explorer-tags-cell></span></td></tr></table>'));
};
</script>${driveScript}${partial}${columnWidths}
<script>
(async () => {
 const assert = (condition, message) => { if (!condition) throw new Error(message); };
 const settle = () => new Promise(resolve => setTimeout(resolve, 0));
 const boxes = [...document.querySelectorAll('.my-drive-item-select')];
 const select = (one, two) => { boxes[0].checked=one; boxes[1].checked=two; boxes[0].dispatchEvent(new Event('change',{bubbles:true})); };
 const submit = id => form.requestSubmit(document.getElementById(({apply:'my-drive-apply-tag-button', remove:'my-drive-remove-tag-button'})[id] || id));
 const status = document.getElementById('explorer-save-status');
 const scroll = document.querySelector('.boreal-permissions-scroll');
 assert(scroll.clientHeight <= 200 && scroll.scrollHeight > scroll.clientHeight, 'permission entries must scroll within 200px');
 assert(document.querySelector('[data-page-step="-1"]').disabled, 'first page has no Previous');
 assert(!document.querySelector('[data-page-step="1"]').disabled, 'Next is enabled');
 const pageCheckbox = document.getElementById('my-drive-select-all');
 const allMatches = document.querySelector('[data-explorer-all-matches]');
 assert(allMatches.closest('label').hidden, 'all matches starts hidden');
 pageCheckbox.click();
 assert(boxes.every(box=>box.checked) && !allMatches.closest('label').hidden, 'header selects page and reveals all matches');
 const labelBounds = allMatches.closest('label').getBoundingClientRect(), headerBounds = allMatches.closest('th').getBoundingClientRect();
 assert(headerBounds.width >= 136 && labelBounds.right <= headerBounds.right && labelBounds.left >= headerBounds.left, 'all matches fits inside restored narrow selection column');
 assert(!document.getElementById('my-drive-migrate-button').disabled, 'real Drive selection enables migration');
 allMatches.click();
 assert(allMatches.checked && document.getElementById('my-drive-selection-count').textContent === '60 selected (all matches)', 'all matches updates selection count');
 allMatches.click();
 assert(boxes.every(box=>box.checked) && !allMatches.checked, 'unchecking all matches keeps current page selected');
 allMatches.click();
 select(false,true);
 assert(!allMatches.checked && allMatches.closest('label').hidden && pageCheckbox.indeterminate, 'manual deselection clears all matches');
 pageCheckbox.click(); allMatches.click();
 form.elements.tag.selectedIndex = -1;
 boxes[0].dataset.isDeleted = 'true';
 submit('my-drive-migrate-button');
 assert(posts.length === 0 && status.getAttribute('role') === 'alert' && status.textContent.includes('Deleted items'), 'deleted migration selections show an actionable alert');
 delete boxes[0].dataset.isDeleted;
 submit('my-drive-migrate-button');
 const progress = document.getElementById('explorer-migration-progress');
 assert(progress.open, 'migration progress modal opens immediately');
 assert(posts[0].get('all_matching') === 'true' && posts[0].get('tag') === 'filtered-tag' && posts[0].get('owner_identity_tag') === 'owner-tag', 'migration submits all matching filters');
 submit('my-drive-migrate-button'); assert(posts.length === 1, 'duplicate migration clicks do not create another plan');
 completions.shift().resolve(new Response('Selected source is unavailable', {status:400})); await settle(); await settle();
 assert(!progress.open && status.getAttribute('role') === 'alert' && status.classList.contains('alert-danger'), 'migration failure closes modal and shows error alert');
 assert(status.textContent.includes('Selected source is unavailable'), 'server error is readable');
 posts.length = 0;
 form.elements.tag.selectedIndex = 0;
 submit('apply');
 assert(posts.length === 1 && posts[0].get('selected_item_ids') === 'one,two' && posts[0].get('all_matching') === 'true', 'first operation captures first selection');
 assert(document.querySelector('.boreal-pending-tag')?.textContent === 'Review', 'optimistic tag appears immediately');
 select(false,true); submit('remove');
 assert(posts.length === 1, 'second write waits for first');
 submit('my-drive-migrate-button');
 assert(posts.length === 1 && status.getAttribute('role') === 'alert', 'pending writes block migration with an alert');
 const filterEvent = new Event('submit',{bubbles:true,cancelable:true}); document.getElementById('filter').dispatchEvent(filterEvent);
 assert(filterEvent.defaultPrevented, 'filter navigation blocked during saves');
 completions.shift().resolve(new Response(null,{status:204})); await settle();
 assert(posts.length === 2 && posts[1].get('selected_item_ids') === 'two', 'queued operation keeps its own IDs');
 completions.shift().resolve(new Response(null,{status:204})); await settle(); await settle();
 assert(status.textContent.startsWith('Tags saved'), 'successful writes reported');
 assert(boxes[1].checked && !boxes[0].checked, 'selection survives reconciliation');
 pageCheckbox.click();
 assert(allMatches.disabled && !allMatches.checked, 'stale filters require refresh before another all-page selection');
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
