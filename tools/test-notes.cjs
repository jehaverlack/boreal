// Generate a self-contained browser regression fixture (no live service required).
const fs = require('node:fs');
const partial = fs.readFileSync('tmpl/html/partials/notes.html', 'utf8');
const setup = `<script>
let notes=[], calls=0, failSave=false;
window.fetch=async(path,options)=>{
 const data=JSON.parse(options.body);calls++;
 if(path==='/notes/list') return {ok:true,json:async()=>data.map(target=>({target,notes}))};
 if(failSave)return {ok:false,text:async()=> 'Test save failure'};
 if(path==='/notes/delete')notes=notes.filter(n=>n.id!==data.id);
 else if(data.id)notes=notes.map(n=>n.id===data.id?{...n,body:data.body,html:'<p>'+data.body+'</p>',revision:n.revision+1}:n);
 else notes.push({id:notes.length+1,body:data.body,html:'<p>'+data.body+'</p>',revision:1});
 return {ok:true,json:async()=>({target:data.target,notes})};
};
</script>`;
const test = `<script>
(async()=>{
 const assert=(ok,message)=>{if(!ok)throw Error(message)};
 try{
 await window.borealNotesReady;
 const cells=[...document.querySelectorAll('[data-note-kind]')],dialog=document.getElementById('boreal-note-dialog'),body=document.getElementById('boreal-note-body'),form=document.getElementById('boreal-note-form');
 const submit=async()=>{form.dispatchEvent(new Event('submit',{cancelable:true,bubbles:true}));await new Promise(r=>setTimeout(r,0))};
 cells[0].querySelector('[aria-label="Add note"]').click();assert(dialog.open,'Add opens editor');assert(document.getElementById('boreal-note-delete').hidden,'Delete hidden for new notes');body.value='First note';await submit();assert(!dialog.open,'Save closes editor');
 assert(cells.every(c=>c.textContent.includes('First note')),'Duplicate rows synchronize');
 cells[0].querySelector('[aria-label="Add note"]').click();body.value='Second note';await submit();assert(cells[0].querySelectorAll('.boreal-note').length===2,'Multiple notes');
 cells[0].querySelector('[aria-label="Edit note"]').click();assert(body.value==='First note','Edit retains Markdown');body.value='Updated note';await submit();assert(cells[1].textContent.includes('Updated note'),'Edit updates both rows');
 cells[0].querySelector('[aria-label="Add note"]').click();body.value='Keep draft';failSave=true;await submit();assert(dialog.open && body.value==='Keep draft','Error retains draft');assert(!document.getElementById('boreal-note-error').hidden,'Error is visible');
 document.getElementById('boreal-note-cancel').click();assert(!dialog.open,'Cancel closes editor');
 const remove=document.getElementById('boreal-note-delete');
 cells[0].querySelector('[aria-label="Edit note"]').click();assert(!remove.hidden,'Delete visible for existing notes');
 remove.click();await new Promise(r=>setTimeout(r,0));assert(dialog.open && !document.getElementById('boreal-note-error').hidden,'Delete error keeps editor open');assert(notes.length===2,'Failed delete preserves notes');
 failSave=false;body.value='';remove.click();assert(remove.disabled,'Delete disabled while pending');await new Promise(r=>setTimeout(r,0));assert(!dialog.open,'Delete closes editor');assert(cells.every(c=>c.querySelectorAll('.boreal-note').length===1 && c.textContent.includes('Second note')),'Delete synchronizes duplicates and preserves other notes');
 document.getElementById('result').textContent='PASS: add, multiple notes, edit, duplicate rows, error retention, cancel, delete';
 }catch(error){document.getElementById('result').textContent='FAIL: '+error.stack}
})();
</script>`;
fs.writeFileSync('/tmp/boreal-explorer-test.html', '<!doctype html><html><body><table><tr><td data-note-kind="drive" data-note-key="one"></td><td data-note-kind="drive" data-note-key="one"></td></tr></table><pre id="result">PENDING</pre>'+setup+partial+test+'</body></html>');
console.log('Notes browser fixture: /tmp/boreal-explorer-test.html');
