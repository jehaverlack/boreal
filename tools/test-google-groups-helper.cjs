const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const crypto = require('node:crypto');
let groups = [];
const context = vm.createContext({
 Session: {getEffectiveUser: () => ({getEmail: () => 'me@example.test'})},
 GroupsApp: {getGroups: () => [...groups]},
 Utilities: {DigestAlgorithm:{SHA_256:'sha256'},computeDigest:(_,str)=>crypto.createHash('sha256').update(str).digest(),base64EncodeWebSafe:b=>Buffer.from(b).toString('base64url')},
});
vm.runInContext(fs.readFileSync('tools/google-groups/Code.gs','utf8'),context);
const group = (email, failure) => ({getEmail:()=>email,getUsers:()=>{if(failure)throw new Error(failure);return [{getEmail:()=> 'user@example.test'}];},getRoles:()=>['BANNED'],getGroups:()=>[{getEmail:()=> 'child@example.test'}],getRole:()=> 'PENDING'});
groups=[group('team@example.test')];
let result=context.borealMyGroups(null);
assert.equal(result.account,'me@example.test');assert.equal(result.groups[0].self_role,'PENDING');assert.equal(result.groups[0].members[0].role,'BANNED');assert.equal(result.groups[0].members[1].type,'GROUP');assert.equal(result.next,null);
groups=[group('hidden@example.test','You do not have permission to view members')];
result=context.borealMyGroups(null);assert.equal(result.groups[0].members_unavailable,true);assert.equal(result.groups[0].members.length,0);
groups=[group('quota@example.test','Service invoked too many times')];assert.throws(()=>context.borealMyGroups(null),/BOREAL_GROUPS_QUERY_FAILED/);
groups=Array.from({length:12},(_,i)=>group(`team${i}@example.test`));
result=context.borealMyGroups(null);assert.equal(result.groups.length,10);assert.equal(context.borealMyGroups(result.next).groups.length,2);
groups.push(group('new@example.test'));assert.throws(()=>context.borealMyGroups(result.next),/BOREAL_GROUPS_CHANGED/);
assert.throws(()=>context.borealMyGroups({offset:-1}),/BOREAL_GROUPS_CHANGED/);
console.log('PASS: My Groups identity, roles, visibility, batching, changed-list protection and quota failures.');
