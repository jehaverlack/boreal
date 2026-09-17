// Generate a browser/PDF fixture from the real Drive table and shared styles.
// All notes are mocked; no live services or inventory data are accessed.
const fs = require('node:fs');
const section = require('./test-template-section.cjs');
const bootstrap = process.env.BOREAL_BOOTSTRAP_CSS || '/tmp/boreal-setup-preview/bootstrap.css';
if (!fs.existsSync(bootstrap)) throw Error('Set BOREAL_BOOTSTRAP_CSS to a local copy of Bootstrap 5.3.8 CSS.');
const bootstrapUrl = require('node:url').pathToFileURL(bootstrap).href;
const read = name => fs.readFileSync(`tmpl/html/${name}`, 'utf8');
const template = read('my-drive.html');
const table = template.slice(template.indexOf('<table data-column-widths="my-drive"'), template.indexOf('</table>') + 8);
const header = table.slice(0, table.indexOf('<tbody>') + 7);
const start = table.indexOf('<tr class=', table.indexOf('<tbody>'));
const row = table.slice(start, table.indexOf('</tr>', start) + 5);
const name = 'NAME-BEGIN-' + 'LongUnbrokenFilename'.repeat(12) + '-NAME-END.docx';
const clean = html => html.replace(/{%[\s\S]*?%}/g, '').replace(/{{\s*([^}]+)\s*}}/g, (_, key) => ({
 'row.name': name, 'row.item_id': 'one', 'row.drive_url': 'https://drive.google.com/file/d/one/view',
 'row.name_url': 'https://drive.google.com/file/d/one/view', 'row.mime_type': 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
 'row.size': '5.2 MB', 'row.modified_at': '2026-09-17 12:34:56', 'row.owner.label': 'owner@example.org',
 'permission.label': 'LONG-PERMISSION-' + 'verylongemail'.repeat(10) + '@example.org', 'tag.name': 'Review',
 'tag.color': '#eeeeee', 'tag.text_color': '#111111'
}[key.trim()] || ''));
const notes = read('partials/notes.html');
const styles = section(read('base.html'), 'style');
const setup = `<script>
let releaseNotes; window.printed = false;
window.print = () => { window.printed = true; };
window.fetch = () => new Promise(resolve => {releaseNotes = () => resolve(new Response(JSON.stringify([{target:{kind:'drive',key:'one'},notes:[{id:1,revision:1,body:'note',html:'<p>NOTE-BEGIN</p>' + Array.from({length:85},(_,i)=>'<p>Note paragraph '+i+' with full text that must remain readable across pages.</p>').join('') + '<p>NOTE-END</p>'},{id:2,revision:1,body:'note2',html:'<p>SECOND-NOTE-END</p>'}]}])))});
</script>`;
const checks = `<script>
(async()=>{
 const assert=(ok,msg)=>{if(!ok)throw Error(msg)};
 document.getElementById('print').click();await Promise.resolve();assert(!window.printed,'printing waits for notes');
 releaseNotes();await window.borealNotesReady;await Promise.resolve();
 assert(window.printed,'printing resumes after notes load');
 const table=document.querySelector('table'),link=document.querySelector('.boreal-name-link'),list=document.querySelector('.boreal-notes-list');
 for(const width of [1267,980]) {
  document.querySelector('main').style.width=width+'px';
  assert(table.getBoundingClientRect().width<=width+1,'table fits printable width');
  assert(link.getBoundingClientRect().width>width*.2,'name has readable column width');
  assert(link.scrollWidth<=link.clientWidth+1,'long name wraps fully');
  assert(list.scrollHeight<=list.clientHeight+1,'all notes visible without scrolling');
  assert(list.getBoundingClientRect().width>width*.15,'notes have a dedicated column');
 }
 document.querySelector('main').style.width='';
 assert(getComputedStyle(document.querySelector('.boreal-col-select')).display==='none','selection column hidden');
 assert(getComputedStyle(document.querySelector('thead input')).display==='none','filters hidden');
 document.body.dataset.test='PASS';
})().catch(error=>{document.body.dataset.test='FAIL';document.getElementById('result').textContent=error.stack});
</script>`;
fs.writeFileSync('/tmp/boreal-print-report-test.html', `<!doctype html><html><head><link rel="stylesheet" href="${bootstrapUrl}"><style>${styles}</style></head><body class="boreal-print-report"><main><div class="boreal-print-actions"><button id="print" onclick="window.print()">Print / Save PDF</button></div><h1>Drive PDF regression</h1><div class="card boreal-explorer-card"><div class="table-responsive boreal-explorer-table">${clean(header + row + '</tbody></table>')}</div></div></main><pre id="result"></pre>${setup}${notes}${checks}</body></html>`);
console.log('/tmp/boreal-print-report-test.html');
