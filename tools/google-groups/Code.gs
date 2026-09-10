/** Boreal My Groups helper. Deploy as an API executable, never as an owner-run web app. */
function borealMyGroups(cursor) {
  const account = Session.getEffectiveUser().getEmail();
  if (!account) throw new Error('BOREAL_IDENTITY_UNAVAILABLE');
  const all = GroupsApp.getGroups().sort((a, b) => a.getEmail().localeCompare(b.getEmail()));
  const digest = Utilities.base64EncodeWebSafe(Utilities.computeDigest(
    Utilities.DigestAlgorithm.SHA_256, all.map(g => g.getEmail()).join('\n')));
  const offset = cursor && cursor.offset || 0;
  if (!Number.isSafeInteger(offset) || offset < 0 || offset > all.length ||
      (cursor && cursor.digest !== digest)) throw new Error('BOREAL_GROUPS_CHANGED');
  const end = Math.min(offset + 10, all.length);
  const groups = all.slice(offset, end).map(group => {
    const result = {email: group.getEmail(), members: [], members_unavailable: false, self_role: 'UNKNOWN'};
    try {
      const users = group.getUsers();
      const roles = users.length ? group.getRoles(users) : [];
      result.members = users.map((user, i) => ({email: user.getEmail(), role: String(roles[i]), type: 'USER'}));
      group.getGroups().forEach(child => result.members.push({email: child.getEmail(), role: 'UNKNOWN', type: 'GROUP'}));
      result.self_role = String(group.getRole(account));
    } catch (error) {
      // Only visibility failures become unavailable lists. Quotas and unexpected
      // failures abort the snapshot, retaining Boreal's last successful inventory.
      if (!/permission|not authorized|access denied|not allowed|not permitted|does not exist/i.test(String(error))) {
        throw new Error('BOREAL_GROUPS_QUERY_FAILED');
      }
      result.members = [];
      result.members_unavailable = true;
    }
    return result;
  });
  return {schema: 1, account: account, groups: groups,
    next: end < all.length ? {offset: end, digest: digest} : null};
}
