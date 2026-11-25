export interface Options {
  /** Parse JSX (default true). */
  jsx?: boolean;
  /** Parse Flow type syntax (default true). */
  types?: boolean;
  components?: boolean;
  enums?: boolean;
  decorators?: boolean;
  earlyErrors?: boolean;
  topLevelAwait?: boolean;
  allowReturnOutsideFunction?: boolean;
  /** Accept syntax Flow's parser lacks (string export names, `#x in o`, JSX element attribute values). */
  esExtras?: boolean;
  /** Parse Flow comment types (`/*: T *​/`). */
  commentTypes?: boolean;
  /** Flow's stricter `oxc_type_feature_parser_rules`. */
  oxcTypeRules?: boolean;
  /** Nesting limit (default 1536 in the WebAssembly build). */
  maxDepth?: number;
  /** Attach `leadingComments` / `trailingComments` to nodes (experimental). */
  internComments?: boolean;
  /** Add a `tokens` array to the program. */
  tokens?: boolean;
  /** Add line/column `loc` to every node. */
  loc?: boolean;
  /** Flow's native tree flavor instead of the Babel-compatible one. */
  flowNative?: boolean;
  /** Report offsets in UTF-16 code units (JavaScript string indices) instead of bytes. */
  utf16?: boolean;
  /** Drop `start`/`end`. */
  noLoc?: boolean;
}

export interface Position { line: number; column: number }

export interface Diagnostic {
  message: string;
  hint: string | null;
  /** Byte offset. */
  pos: number;
  loc: Position;
}

export interface Node { type: string; start?: number; end?: number; [key: string]: unknown }
export interface Program extends Node {
  type: 'Program';
  body: Node[];
  comments: Array<{ type: 'Line' | 'Block'; value: string; start?: number; end?: number }>;
  tokens?: Array<{ type: string; value: string; start?: number; end?: number }>;
}

export class PrysmiteSyntaxError extends SyntaxError {
  loc: Position;
  pos: number;
  hint: string | null;
  diagnostics: Diagnostic[];
}

export function parse(source: string, options?: Options): Program;
export function parseRecovering(source: string, options?: Options): { ast: Program; errors: Diagnostic[] };
export function tokenize(source: string, options?: Options): NonNullable<Program['tokens']>;

export interface ImportInfo {
  kind: 'import' | 'export-from' | 'dynamic-import' | 'require';
  source: string;
  importKind?: string | null;
  specifiers?: Array<{ kind: 'default' | 'namespace' | 'named'; imported: string; local: string }>;
  start: number;
  end: number;
}
export interface TypeInfo { kind: string; name: string | null; exported: boolean; start: number; end: number }
export interface ScopeReport {
  bindings: number;
  unused: Array<{ name: string; kind: string; start: number; end: number }>;
  undeclared: Array<{ name: string; uses: Array<{ start: number; end: number }> }>;
}

export function analyze(source: string, what: 'imports' | 'types' | 'features' | 'scope' | 'query', arg?: string, options?: Options): unknown;
export function imports(source: string, options?: Options): ImportInfo[];
export function types(source: string, options?: Options): TypeInfo[];
export function features(source: string, options?: Options): Record<string, number>;
export function scope(source: string, options?: Options): ScopeReport;
export function query(source: string, selector: string, options?: Options): Array<{ type: string; start: number; end: number }>;
export function stripTypes(source: string, options?: Options): { code: string; unsupported: Array<{ kind: string; start: number }> };
export function version(): string;
