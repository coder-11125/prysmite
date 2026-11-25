// Shared reference options. The npm `flow-parser` is a Babel-compatible wrapper: it only understands
// `flow: 'all'` and the `enable*` names below (Flow's own `types`, `enums`, ... option names are ignored).
const fs = require('fs');
const MAP = {
  enums: 'enableEnums',
  components: 'enableExperimentalComponentSyntax',
  pattern_matching: 'enableExperimentalFlowMatchSyntax',
  records: 'enableExperimentalFlowRecordSyntax',
  esproposal_decorators: 'enableExperimentalDecorators',
};
function fixtureOptions(file) {
  let ex = {};
  try { ex = JSON.parse(fs.readFileSync(file.replace(/\.js$/, '.options.json'), 'utf8')); } catch (e) {}
  return ex;
}
// The npm wrapper can't switch Flow syntax off, so fixtures that do are checked against the
// expected result recorded next to them (`*.tree.json`, produced by Flow's own test runner).
function skip(file) {
  const ex = fixtureOptions(file);
  return ex.types === false || ex.components === false || ex.esproposal_decorators === false || ex.enums === false || ex.oxc_type_feature_parser_rules === true;
}
function cliFlags(file) {
  const ex = fixtureOptions(file);
  const flags = [];
  if (ex.types === false) flags.push('--no-types');
  if (ex.components === false) flags.push('--no-components');
  if (ex.esproposal_decorators === false) flags.push('--no-decorators');
  if (ex.enums === false) flags.push('--no-enums');
  if (ex.oxc_type_feature_parser_rules) flags.push('--oxc-type-rules');
  return flags;
}
function recordedTree(file) {
  try { return JSON.parse(fs.readFileSync(file.replace(/\.js$/, '.tree.json'), 'utf8')); } catch (e) { return null; }
}
function flowOptions(file) {
  const o = { flow: 'all', tokens: false };
  if (file) {
    const ex = fixtureOptions(file);
    for (const k in MAP) if (typeof ex[k] === 'boolean') o[MAP[k]] = ex[k];
  }
  return o;
}
module.exports = { flowOptions, skip, fixtureOptions, cliFlags, recordedTree };
