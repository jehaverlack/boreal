// Generate a self-checking Chromium fixture, then open /tmp/boreal-column-widths-test.html.
// chromium --headless --no-sandbox --user-data-dir=/tmp/boreal-column-browser --dump-dom file:///tmp/boreal-column-widths-test.html
const fs = require('node:fs');
const path = require('node:path');
const templateSection = require('./test-template-section.cjs');
const partial = fs.readFileSync(path.join(__dirname, '../tmpl/html/partials/column-widths.html'), 'utf8');
const base = fs.readFileSync(path.join(__dirname, '../tmpl/html/base.html'), 'utf8');
const script = templateSection(partial, 'script');
const styles = templateSection(base, 'style') + templateSection(partial, 'style');
const markup = `<input id="filter" value="keep,!review"><input id="sort" value="modified">
${['drive', 'keeper'].map(key => `<div class="table-responsive boreal-explorer-table" style="height:200px;width:1100px"><table data-column-widths="${key}" style="width:100%">
${key === 'drive' ? '<colgroup><col style="width:45px"><col style="width:40%"><col></colgroup>' : ''}
<thead><tr><th><input type="checkbox"></th><th><button type="button">Name</button><input value="search"></th><th>Permissions</th></tr></thead>
<tbody><tr><td></td><td>A long item title and some metadata</td><td>Reader</td></tr></tbody></table></div>`).join('')}`;
const checks = `
const assert = (value, message) => { if (!value) throw new Error(message); };
const init = () => { document.body.innerHTML = markup; new Function(source)(); };
const table = key => document.querySelector('[data-column-widths="'+key+'"]');
const handle = (key, index=1) => table(key).querySelectorAll('[role="separator"]')[index];
const width = key => table(key).tHead.rows[0].cells[1].getBoundingClientRect().width;
const press = (key, direction, shift=false) => handle(key).dispatchEvent(new KeyboardEvent('keydown',{key:direction,shiftKey:shift,bubbles:true}));
try {
    localStorage.clear(); init();
    assert(document.querySelectorAll('[role="separator"]').length===6, 'handles on all columns');
    const before = width('drive'); const other = width('keeper');
    press('drive','ArrowRight',true);
    assert(Math.abs(width('drive')-before-50)<3,'keyboard increases actual column width');
    assert(width('keeper')===other,'separate explorer unchanged');
    const persisted = width('drive'); init();
    assert(Math.abs(width('drive')-persisted)<1,'saved width restored on navigation/tag refresh');
    let clicks=0; table('drive').tHead.addEventListener('click',()=>clicks++);
    handle('drive').click(); assert(clicks===0,'resize handle does not sort');
    table('drive').querySelector('button').click(); assert(clicks===1,'sort control remains usable');
    const h=handle('drive'); h.setPointerCapture=()=>{};
    const old=width('drive');
    for(const [type,x] of [['pointerdown',100],['pointermove',220],['pointerup',220]]) h.dispatchEvent(new PointerEvent(type,{clientX:x,pointerId:1,button:0,bubbles:true}));
    assert(Math.abs(width('drive')-old-120)<3,'pointer drag increases rendered width');
    assert(!document.body.classList.contains('boreal-resizing'),'drag cleaned up');
    const reset=document.querySelector('.boreal-column-controls button'); reset.click();
    assert(!table('drive').classList.contains('boreal-columns-sized'),'reset restores automatic layout');
    assert(Math.abs(width('drive')-before)<3,'reset restores original column sizing');
    init(); assert(Math.abs(width('drive')-before)<3,'reset removes saved widths');
    for(let i=0;i<50;i++) press('drive','ArrowLeft',true);
    assert(width('drive')>=79 && width('drive')<=83,'minimum width enforced');
    assert(document.getElementById('filter').value==='keep,!review' && document.getElementById('sort').value==='modified','filters and sort preserved');
    document.body.classList.add('boreal-print-report');
    assert(getComputedStyle(handle('drive')).display==='none','print preview hides resize controls');
    assert(Math.abs(width('drive')-before)<3,'print preview ignores saved widths');
    document.body.classList.remove('boreal-print-report');
    for(let i=0;i<localStorage.length;i++) localStorage.setItem(localStorage.key(i),'[null,-10,"bad"]');
    init(); assert(!table('drive').classList.contains('boreal-columns-sized'),'invalid stored widths ignored');
    document.body.innerHTML='<pre>PASS: column resizing, keyboard, persistence, isolation, reset, limits, sorting, filters, print preview and invalid storage</pre>';
} catch(error) { document.body.innerHTML='<pre>FAIL: '+error.message+'</pre>'; }
`;
fs.writeFileSync('/tmp/boreal-column-widths-test.html', `<html><head><style>${styles}</style></head><body><script>const markup=${JSON.stringify(markup)};const source=${JSON.stringify(script)};${checks}</script></body></html>`);
console.log('Created /tmp/boreal-column-widths-test.html');
