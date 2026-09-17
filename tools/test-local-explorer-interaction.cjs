// Run the Rust local_file_explorer_renders test with BOREAL_UI_FIXTURE_DIR first.
// Generates a self-checking browser fixture; all migration requests are mocked.
const fs = require('fs');
const fixture = fs.readFileSync('/tmp/boreal-restored-ui/local-files.html', 'utf8');
const localTemplate = fs.readFileSync('tmpl/html/local-files.html', 'utf8');
const localScript = localTemplate.slice(localTemplate.indexOf('<script>'), localTemplate.indexOf('</script>') + 9);
const partial = fs.readFileSync('tmpl/html/partials/explorer-interaction.html', 'utf8');
let content = fixture.match(/<main\b[^>]*>([\s\S]*?)<\/main>/)[1].replace(/<script\b[^>]*>[\s\S]*?<\/script>/g, '');
// The explorer interaction dialog is included in the real rendered main.
content = content.replace(/<dialog id="explorer-migration-progress"[\s\S]*?<\/dialog>/, '');
const html = `<!doctype html><html><body><main>${content}</main>
<script>
localStorage.clear();
const requests = [];
window.fetch = (url, options = {}) => {
  if (options.method === 'POST') requests.push({url, body:new URLSearchParams(options.body)});
  return Promise.resolve(new Response('Simulated planning error to keep this fixture on the page', {status:400}));
};
</script>${localScript}${partial}
<script>
(async () => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const settle = () => new Promise(resolve => setTimeout(resolve, 0));
  const button = document.getElementById('local-migrate-button');
  const form = document.querySelector('[data-explorer-tags]');
  assert(button.disabled, 'migration starts disabled');
  document.querySelector('.local-select-all').click();
  assert(!button.disabled, 'selection enables migration');
  const all = document.querySelector('[data-explorer-all-matches]');
  assert(!all.closest('label').hidden, 'all matches is available');
  all.click();
  form.requestSubmit(button); await settle(); await settle();
  assert(requests.length === 1 && requests[0].url === '/local-files/migrate', 'local migration endpoint receives request');
  assert(requests[0].body.get('selected_item_ids') === '7', 'selected IDs preserved');
  assert(requests[0].body.get('all_matching') === 'true', 'all pages selection preserved');
  assert(requests[0].body.get('tag') === 'needs-review', 'local tag filter preserved');
  assert(requests[0].body.get('background') === 'true', 'planning runs with progress feedback');
  assert(document.getElementById('explorer-save-status').textContent.includes('Simulated planning error'), 'planning error visible');
  document.getElementById('explorer-clear-selection').click();
  assert(button.disabled, 'clear selection disables migration');
  document.body.dataset.test='PASS';
  const result=document.createElement('pre');result.id='test-result';result.textContent='PASS: local migration selection, all matches, filters, planning errors, and clear selection';document.body.append(result);
})().catch(error => {document.body.dataset.test='FAIL';const result=document.createElement('pre');result.id='test-result';result.textContent=error.stack;document.body.append(result)});
</script></body></html>`;
fs.writeFileSync('/tmp/boreal-local-explorer-test.html',html);
console.log('/tmp/boreal-local-explorer-test.html');
