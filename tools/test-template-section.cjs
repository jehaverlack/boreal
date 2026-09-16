// Extract a literal section from trusted repository templates for test execution.
// This is not an HTML parser or sanitizer and must not process untrusted HTML.
module.exports = function templateSection(source, tag) {
    const opening = `<${tag}>`;
    const closing = `</${tag}>`;
    const start = source.indexOf(opening);
    if (start === -1) throw new Error(`Missing ${opening} in test template`);
    const contentStart = start + opening.length;
    const end = source.indexOf(closing, contentStart);
    if (end === -1) throw new Error(`Missing ${closing} in test template`);
    return source.slice(contentStart, end);
};
