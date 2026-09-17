// Generate the fixture with BOREAL_UI_FIXTURE_DIR=/tmp/boreal-restored-ui cargo test local_file_explorer_renders.
// This browser test edits only a rendered fixture; it never starts a transfer.
const fs = require('fs');
const fixture = fs.readFileSync('/tmp/boreal-restored-ui/migration-conflicts.html', 'utf8');
const main = fixture.match(/<main\b[^>]*>([\s\S]*?)<\/main>/)[1];
const html = `<!doctype html><html><body>${main}<script>
try {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const rows = [...document.querySelectorAll('[data-selection-name]')];
  const boxes = rows.map(row => row.querySelector('.migration-keep'));
  const save = document.getElementById('save-revised-selection');
  const removed = () => JSON.parse(document.getElementById('removed-selection-ids').value);
  const search = document.getElementById('selection-search');
  assert(document.getElementById('migration-selection').open, 'conflict editor opens automatically');
  assert(rows.length === 3 && boxes.every(box => box.checked), 'complete saved selection is checked');
  assert(rows[0].classList.contains('table-warning') && rows[1].classList.contains('table-warning'), 'both same-name items are highlighted despite different sizes');
  assert(rows[2].hidden && boxes[2].checked, 'conflicts-only view retains other selections');
  assert(save.disabled, 'unresolved conflicts prevent saving');
  search.value = '/other/'; search.dispatchEvent(new Event('input'));
  assert(rows[0].hidden && !rows[1].hidden, 'search finds full source paths');
  boxes[1].click();
  assert(!save.disabled && removed().join() === '8', 'removing one conflicting item enables revision');
  assert(!rows[0].classList.contains('table-warning'), 'resolved conflict loses highlight');
  assert(boxes[0].checked && boxes[2].checked, 'hidden selections remain intact');
  boxes[1].click(); assert(save.disabled, 'restoring duplicate blocks saving');
  boxes.forEach(box => { if (box.checked) box.click(); });
  assert(save.disabled && removed().length === 3, 'empty selection cannot be saved');
  boxes[0].click(); assert(!save.disabled, 'single remaining item can be saved');
  assert(document.querySelector('a[href^="/local-files?"]').getAttribute('href') === '/local-files?tag=needs-review&name=Report', 'original filters are preserved');
  document.body.dataset.test = 'PASS';
  const result = document.createElement('pre'); result.id = 'test-result'; result.textContent = 'PASS: conflict highlighting, saved selection, search, filters, and revision validation'; document.body.append(result);
} catch (error) {
  document.body.dataset.test = 'FAIL';
  const result = document.createElement('pre'); result.id = 'test-result'; result.textContent = error.stack; document.body.append(result);
}
</script></body></html>`;
fs.writeFileSync('/tmp/boreal-migration-selection-test.html', html);
console.log('/tmp/boreal-migration-selection-test.html');
