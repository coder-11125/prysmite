//! ESTree-style AST with Flow extensions.
//!
//! Every node is a [`Node`]: a [`Kind`] plus a byte range. `Kind` serializes
//! with an internal `"type"` tag, so the JSON looks like what `flow-parser`
//! and Babel produce (`{ "type": "Identifier", "name": "x", "start": 0, ... }`).

use crate::lexer::Comment;
use serde::Serialize;
use serde_json::Value;

pub type B = Box<Node>;
pub type Opt = Option<Box<Node>>;

#[derive(Debug, Clone, Serialize)]
pub struct Node {
    #[serde(flatten)]
    pub kind: Kind,
    pub start: usize,
    pub end: usize,
}

impl Node {
    pub fn new(kind: Kind, start: usize, end: usize) -> Node {
        Node { kind, start, end }
    }
    pub fn boxed(self) -> B {
        Box::new(self)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum Kind {
    // ───────────────────────────── Program ─────────────────────────────
    Program {
        body: Vec<Node>,
        comments: Vec<Comment>,
        interpreter: Opt,
        source_type: String,
    },
    InterpreterDirective {
        value: String,
    },

    // ──────────────────────────── Statements ───────────────────────────
    ExpressionStatement {
        expression: B,
        directive: Option<String>,
    },
    BlockStatement {
        body: Vec<Node>,
    },
    EmptyStatement,
    DebuggerStatement,
    WithStatement {
        object: B,
        body: B,
    },
    ReturnStatement {
        argument: Opt,
    },
    LabeledStatement {
        label: B,
        body: B,
    },
    BreakStatement {
        label: Opt,
    },
    ContinueStatement {
        label: Opt,
    },
    IfStatement {
        test: B,
        consequent: B,
        alternate: Opt,
    },
    SwitchStatement {
        discriminant: B,
        cases: Vec<Node>,
    },
    SwitchCase {
        test: Opt,
        consequent: Vec<Node>,
    },
    ThrowStatement {
        argument: B,
    },
    TryStatement {
        block: B,
        handler: Opt,
        finalizer: Opt,
    },
    CatchClause {
        param: Opt,
        body: B,
    },
    WhileStatement {
        test: B,
        body: B,
    },
    DoWhileStatement {
        body: B,
        test: B,
    },
    ForStatement {
        init: Opt,
        test: Opt,
        update: Opt,
        body: B,
    },
    ForInStatement {
        left: B,
        right: B,
        body: B,
    },
    ForOfStatement {
        left: B,
        right: B,
        body: B,
        #[serde(rename = "await")]
        is_await: bool,
    },
    VariableDeclaration {
        declarations: Vec<Node>,
        kind: String,
    },
    VariableDeclarator {
        id: B,
        init: Opt,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        definite: bool,
    },

    // ───────────────────── Functions, classes, modules ─────────────────
    FunctionDeclaration {
        id: Opt,
        params: Vec<Node>,
        body: B,
        generator: bool,
        #[serde(rename = "async")]
        is_async: bool,
        type_parameters: Opt,
        return_type: Opt,
        predicate: Opt,
    },
    FunctionExpression {
        id: Opt,
        params: Vec<Node>,
        body: B,
        generator: bool,
        #[serde(rename = "async")]
        is_async: bool,
        type_parameters: Opt,
        return_type: Opt,
        predicate: Opt,
    },
    ArrowFunctionExpression {
        params: Vec<Node>,
        body: B,
        expression: bool,
        #[serde(rename = "async")]
        is_async: bool,
        type_parameters: Opt,
        return_type: Opt,
        predicate: Opt,
    },
    ClassDeclaration {
        id: Opt,
        super_class: Opt,
        super_type_arguments: Opt,
        type_parameters: Opt,
        implements: Vec<Node>,
        body: B,
        decorators: Vec<Node>,
        #[serde(rename = "abstract", skip_serializing_if = "std::ops::Not::not")]
        is_abstract: bool,
    },
    ClassExpression {
        id: Opt,
        super_class: Opt,
        super_type_arguments: Opt,
        type_parameters: Opt,
        implements: Vec<Node>,
        body: B,
        decorators: Vec<Node>,
        #[serde(rename = "abstract", skip_serializing_if = "std::ops::Not::not")]
        is_abstract: bool,
    },
    ClassBody {
        body: Vec<Node>,
    },
    ParameterProperty {
        key: B,
        value: Opt,
        type_annotation: Opt,
        computed: bool,
        #[serde(rename = "static")]
        is_static: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        variance: Opt,
        ts_accessibility: Option<String>,
        declare: bool,
        optional: bool,
        decorators: Vec<Node>,
    },
    DeclareClassExtendsCall {
        callee: B,
        argument: B,
    },
    ClassImplements {
        id: B,
        type_arguments: Opt,
    },
    MethodDefinition {
        key: B,
        value: B,
        kind: String,
        computed: bool,
        #[serde(rename = "static")]
        is_static: bool,
        decorators: Vec<Node>,
        #[serde(rename = "override")]
        is_override: bool,
        ts_accessibility: Option<String>,
    },
    AbstractMethodDefinition {
        key: B,
        value: B,
        computed: bool,
        #[serde(rename = "override")]
        is_override: bool,
        ts_accessibility: Option<String>,
    },
    AbstractPropertyDefinition {
        key: B,
        value: Opt,
        type_annotation: Opt,
        computed: bool,
        variance: Opt,
        #[serde(rename = "override")]
        is_override: bool,
        ts_accessibility: Option<String>,
    },
    Decorator {
        expression: B,
    },
    PropertyDefinition {
        key: B,
        value: Opt,
        computed: bool,
        #[serde(rename = "static")]
        is_static: bool,
        type_annotation: Opt,
        variance: Opt,
        declare: bool,
        ts_accessibility: Option<String>,
        optional: bool,
        definite: bool,
        #[serde(rename = "override")]
        is_override: bool,
        decorators: Vec<Node>,
    },
    StaticBlock {
        body: Vec<Node>,
    },

    ImportDeclaration {
        specifiers: Vec<Node>,
        source: B,
        import_kind: String,
        attributes: Vec<Node>,
        phase: Option<String>,
    },
    ImportEqualsDeclaration {
        id: B,
        module_reference: B,
        import_kind: String,
        is_export: bool,
    },
    ExternalModuleReference {
        expression: B,
    },
    ExportAssignment {
        expression: B,
    },
    NamespaceExportDeclaration {
        id: B,
    },
    ImportAttribute {
        key: B,
        value: B,
    },
    ImportSpecifier {
        imported: B,
        local: B,
        import_kind: Option<String>,
    },
    ImportDefaultSpecifier {
        local: B,
    },
    ImportNamespaceSpecifier {
        local: B,
    },
    ExportNamedDeclaration {
        declaration: Opt,
        specifiers: Vec<Node>,
        source: Opt,
        export_kind: String,
        attributes: Vec<Node>,
    },
    ExportSpecifier {
        local: B,
        exported: B,
        export_kind: String,
    },
    ExportDefaultDeclaration {
        declaration: B,
        export_kind: String,
    },
    ExportAllDeclaration {
        exported: Opt,
        source: B,
        export_kind: String,
        attributes: Vec<Node>,
    },

    // ───────────────────────────── Expressions ─────────────────────────
    Identifier {
        name: String,
        type_annotation: Opt,
        optional: bool,
    },
    PrivateIdentifier {
        name: String,
    },
    Literal {
        value: Value,
        raw: String,
        literal_type: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        regex: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        bigint: Option<String>,
    },
    TemplateLiteral {
        quasis: Vec<Node>,
        expressions: Vec<Node>,
    },
    TemplateElement {
        value: Value,
        tail: bool,
    },
    TaggedTemplateExpression {
        tag: B,
        quasi: B,
        type_arguments: Opt,
    },
    ThisExpression,
    Super,
    ArrayExpression {
        elements: Vec<Option<Node>>,
        trailing_comma: bool,
    },
    OptionalIndexedAccessType {
        object_type: B,
        index_type: B,
        optional: bool,
    },
    ImportType {
        source: B,
    },
    NonNullExpression {
        expression: B,
    },
    ChainExpression {
        expression: B,
    },
    ObjectExpression {
        properties: Vec<Node>,
    },
    Property {
        key: B,
        value: B,
        kind: String,
        method: bool,
        shorthand: bool,
        computed: bool,
    },
    SpreadElement {
        argument: B,
    },
    UnaryExpression {
        operator: String,
        prefix: bool,
        argument: B,
    },
    UpdateExpression {
        operator: String,
        prefix: bool,
        argument: B,
    },
    BinaryExpression {
        operator: String,
        left: B,
        right: B,
    },
    LogicalExpression {
        operator: String,
        left: B,
        right: B,
    },
    AssignmentExpression {
        operator: String,
        left: B,
        right: B,
    },
    ConditionalExpression {
        test: B,
        consequent: B,
        alternate: B,
    },
    CallExpression {
        callee: B,
        arguments: Vec<Node>,
        type_arguments: Opt,
        optional: bool,
    },
    NewExpression {
        callee: B,
        arguments: Vec<Node>,
        type_arguments: Opt,
    },
    MemberExpression {
        object: B,
        property: B,
        computed: bool,
        optional: bool,
    },
    SequenceExpression {
        expressions: Vec<Node>,
    },
    AwaitExpression {
        argument: B,
    },
    YieldExpression {
        argument: Opt,
        delegate: bool,
    },
    MetaProperty {
        meta: B,
        property: B,
    },
    ImportExpression {
        source: B,
        options: Opt,
        phase: Option<String>,
    },

    // ───────────────────────────── Patterns ────────────────────────────
    ObjectPattern {
        properties: Vec<Node>,
        type_annotation: Opt,
    },
    ArrayPattern {
        elements: Vec<Option<Node>>,
        type_annotation: Opt,
    },
    RestElement {
        argument: B,
        type_annotation: Opt,
    },
    AssignmentPattern {
        left: B,
        right: B,
    },

    // ─────────────────────────── Flow: casts ───────────────────────────
    TypeCastExpression {
        expression: B,
        type_annotation: B,
    },
    AsExpression {
        expression: B,
        type_annotation: B,
    },
    AsConstExpression {
        expression: B,
    },

    // ─────────────────────────── Flow: types ───────────────────────────
    TypeAnnotation {
        type_annotation: B,
    },
    AnyTypeAnnotation,
    MixedTypeAnnotation,
    EmptyTypeAnnotation,
    NumberTypeAnnotation,
    StringTypeAnnotation,
    BooleanTypeAnnotation,
    VoidTypeAnnotation,
    NullLiteralTypeAnnotation,
    SymbolTypeAnnotation,
    BigIntTypeAnnotation,
    ExistsTypeAnnotation,
    ThisTypeAnnotation,
    ConditionalTypeAnnotation {
        check_type: B,
        extends_type: B,
        true_type: B,
        false_type: B,
    },
    InferTypeAnnotation {
        type_parameter: B,
    },
    KeyofTypeAnnotation {
        argument: B,
    },
    TypeOperator {
        operator: String,
        type_annotation: B,
    },
    TypePredicate {
        parameter_name: B,
        type_annotation: Opt,
        kind: Option<String>,
    },
    InterfaceTypeAnnotation {
        extends: Vec<Node>,
        body: B,
    },
    SatisfiesExpression {
        expression: B,
        type_annotation: B,
    },
    ObjectTypeMappedTypeProperty {
        key_tparam: B,
        prop_type: B,
        source_type: B,
        name_type: Opt,
        variance: Opt,
        variance_op: Option<String>,
        optional: Option<String>,
    },
    NeverTypeAnnotation,
    UnknownTypeAnnotation,
    UndefinedTypeAnnotation,
    NullableTypeAnnotation {
        type_annotation: B,
    },
    ArrayTypeAnnotation {
        element_type: B,
    },
    IndexedAccessType {
        object_type: B,
        index_type: B,
    },
    TupleTypeAnnotation {
        element_types: Vec<Node>,
        inexact: bool,
    },
    TupleTypeLabeledElement {
        label: B,
        element_type: B,
        variance: Opt,
        optional: bool,
    },
    TupleTypeSpreadElement {
        label: Opt,
        type_annotation: B,
    },
    UnionTypeAnnotation {
        types: Vec<Node>,
    },
    IntersectionTypeAnnotation {
        types: Vec<Node>,
    },
    TypeofTypeAnnotation {
        argument: B,
        type_arguments: Opt,
    },
    QualifiedTypeofIdentifier {
        qualification: B,
        id: B,
    },
    GenericTypeAnnotation {
        id: B,
        type_arguments: Opt,
    },
    QualifiedTypeIdentifier {
        qualification: B,
        id: B,
    },
    StringLiteralTypeAnnotation {
        value: String,
        raw: String,
    },
    NumberLiteralTypeAnnotation {
        value: Value,
        raw: String,
    },
    BigIntLiteralTypeAnnotation {
        value: Value,
        bigint: String,
        raw: String,
    },
    BooleanLiteralTypeAnnotation {
        value: bool,
        raw: String,
    },
    FunctionTypeAnnotation {
        params: Vec<Node>,
        rest: Opt,
        #[serde(rename = "this")]
        this_param: Opt,
        return_type: Opt,
        type_parameters: Opt,
    },
    ConstructorTypeAnnotation {
        #[serde(rename = "abstract")]
        is_abstract: bool,
        params: Vec<Node>,
        rest: Opt,
        return_type: Opt,
        type_parameters: Opt,
    },
    TemplateLiteralTypeAnnotation {
        quasis: Vec<Node>,
        types: Vec<Node>,
    },
    TupleTypeElement {
        element_type: B,
        optional: bool,
    },
    FunctionTypeParam {
        name: Opt,
        type_annotation: B,
        optional: bool,
    },
    ObjectTypeAnnotation {
        properties: Vec<Node>,
        indexers: Vec<Node>,
        call_properties: Vec<Node>,
        internal_slots: Vec<Node>,
        exact: bool,
        inexact: bool,
    },
    ObjectTypeProperty {
        key: B,
        value: Opt,
        optional: bool,
        #[serde(rename = "static")]
        is_static: bool,
        proto: bool,
        method: bool,
        variance: Opt,
        kind: String,
        computed: bool,
        #[serde(rename = "tsAccessibility", skip_serializing_if = "Option::is_none")]
        ts_accessibility: Option<String>,
        #[serde(rename = "override", skip_serializing_if = "std::ops::Not::not")]
        is_override: bool,
        #[serde(rename = "abstract", skip_serializing_if = "std::ops::Not::not")]
        is_abstract: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        init: Opt,
    },
    ObjectTypePrivateField {
        key: B,
    },
    ObjectTypeSpreadProperty {
        argument: B,
    },
    ObjectTypeIndexer {
        id: Opt,
        key: B,
        value: B,
        variance: Opt,
        optional: bool,
        #[serde(rename = "static")]
        is_static: bool,
    },
    ObjectTypeCallProperty {
        value: B,
        #[serde(rename = "static")]
        is_static: bool,
    },
    ObjectTypeInternalSlot {
        id: B,
        value: B,
        optional: bool,
        #[serde(rename = "static")]
        is_static: bool,
        method: bool,
    },
    ComponentDeclaration {
        id: B,
        params: Vec<Node>,
        body: B,
        renders_type: Opt,
        type_parameters: Opt,
        #[serde(rename = "async")]
        is_async: bool,
    },
    ComponentParameter {
        name: B,
        local: B,
        shorthand: bool,
    },
    DeclareComponent {
        id: B,
        params: Vec<Node>,
        rest: Opt,
        renders_type: Opt,
        type_parameters: Opt,
        implicit_declare: bool,
    },
    ComponentTypeAnnotation {
        params: Vec<Node>,
        rest: Opt,
        renders_type: Opt,
        type_parameters: Opt,
    },
    ComponentTypeParameter {
        name: Opt,
        type_annotation: B,
        optional: bool,
    },
    HookDeclaration {
        id: B,
        params: Vec<Node>,
        body: B,
        return_type: Opt,
        type_parameters: Opt,
        #[serde(rename = "async")]
        is_async: bool,
    },
    DeclareHook {
        id: B,
    },
    HookTypeAnnotation {
        params: Vec<Node>,
        rest: Opt,
        return_type: B,
        type_parameters: Opt,
    },
    DeclareNamespace {
        global: bool,
        id: B,
        body: B,
        keyword: String,
    },
    // ── match expressions / statements and their patterns ──
    MatchExpression {
        argument: B,
        cases: Vec<Node>,
    },
    MatchExpressionCase {
        pattern: B,
        body: B,
        guard: Opt,
    },
    MatchStatement {
        argument: B,
        cases: Vec<Node>,
    },
    MatchStatementCase {
        pattern: B,
        body: B,
        guard: Opt,
    },
    MatchWildcardPattern,
    MatchLiteralPattern {
        literal: B,
    },
    MatchUnaryPattern {
        operator: String,
        argument: B,
    },
    MatchIdentifierPattern {
        id: B,
    },
    MatchMemberPattern {
        base: B,
        property: B,
    },
    MatchBindingPattern {
        id: B,
        kind: String,
    },
    MatchObjectPattern {
        properties: Vec<Node>,
        rest: Opt,
    },
    MatchObjectPatternProperty {
        key: B,
        pattern: B,
        shorthand: bool,
    },
    MatchArrayPattern {
        elements: Vec<Node>,
        rest: Opt,
    },
    MatchRestPattern {
        argument: Opt,
    },
    MatchOrPattern {
        patterns: Vec<Node>,
    },
    MatchAsPattern {
        pattern: B,
        target: B,
    },
    MatchInstancePattern {
        target_constructor: B,
        properties: B,
    },
    MatchInstanceObjectPattern {
        properties: Vec<Node>,
        rest: Opt,
    },
    // ── records ──
    RecordDeclaration {
        id: B,
        type_parameters: Opt,
        implements: Vec<Node>,
        body: B,
    },
    RecordDeclarationBody {
        elements: Vec<Node>,
    },
    RecordDeclarationProperty {
        key: B,
        type_annotation: B,
        default_value: Opt,
    },
    RecordDeclarationStaticProperty {
        key: B,
        type_annotation: B,
        value: Opt,
    },
    RecordDeclarationImplements {
        id: B,
        type_arguments: Opt,
    },
    RecordExpression {
        record_constructor: B,
        type_arguments: Opt,
        properties: B,
    },
    RecordExpressionProperties {
        properties: Vec<Node>,
    },
    InferredPredicate,
    DeclaredPredicate {
        value: B,
    },
    TypeParameterDeclaration {
        params: Vec<Node>,
    },
    TypeParameterInstantiation {
        params: Vec<Node>,
    },
    TypeParameter {
        name: String,
        #[serde(rename = "const")]
        is_const: bool,
        bound: Opt,
        variance: Opt,
        default: Opt,
        uses_extends_bound: bool,
    },
    Variance {
        kind: String,
    },

    // ───────────────────── Flow: declarations ──────────────────────────
    TypeAlias {
        id: B,
        type_parameters: Opt,
        right: B,
    },
    OpaqueType {
        id: B,
        type_parameters: Opt,
        supertype: Opt,
        impltype: Opt,
        lower_bound: Opt,
        upper_bound: Opt,
    },
    InterfaceDeclaration {
        id: B,
        type_parameters: Opt,
        extends: Vec<Node>,
        body: B,
    },
    InterfaceExtends {
        id: B,
        type_arguments: Opt,
    },
    DeclareVariable {
        declarations: Vec<Node>,
        kind: String,
        implicit_declare: bool,
    },
    DeclareFunction {
        id: B,
        predicate: Opt,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        implicit_declare: bool,
    },
    DeclareMethodDefinition {
        key: B,
        value: B,
        #[serde(rename = "static")]
        is_static: bool,
        optional: bool,
        computed: bool,
        kind: Option<String>,
        #[serde(rename = "override")]
        is_override: bool,
    },
    DeclareClass {
        id: B,
        type_parameters: Opt,
        extends: Vec<Node>,
        mixins: Vec<Node>,
        implements: Vec<Node>,
        body: B,
        #[serde(rename = "abstract", skip_serializing_if = "std::ops::Not::not")]
        is_abstract: bool,
        implicit_declare: bool,
    },
    DeclareTypeAlias {
        id: B,
        type_parameters: Opt,
        right: B,
    },
    DeclareOpaqueType {
        id: B,
        type_parameters: Opt,
        supertype: Opt,
        impltype: Opt,
        lower_bound: Opt,
        upper_bound: Opt,
        implicit_declare: bool,
    },
    DeclareInterface {
        id: B,
        type_parameters: Opt,
        extends: Vec<Node>,
        body: B,
    },
    DeclareModule {
        id: B,
        body: B,
    },
    DeclareModuleExports {
        type_annotation: B,
    },
    DeclareExportDeclaration {
        default: bool,
        declaration: Opt,
        specifiers: Vec<Node>,
        source: Opt,
        implicit_declare: bool,
    },
    DeclareExportAllDeclaration {
        source: B,
    },
    DeclareEnum {
        id: B,
        body: B,
        implicit_declare: bool,
        #[serde(rename = "const", skip_serializing_if = "std::ops::Not::not")]
        is_const: bool,
    },
    EnumDeclaration {
        id: B,
        body: B,
        #[serde(rename = "const", skip_serializing_if = "std::ops::Not::not")]
        is_const: bool,
    },
    EnumBody {
        members: Vec<Node>,
        explicit_type: Option<String>,
        has_unknown_members: bool,
    },
    EnumDefaultedMember {
        id: B,
    },
    EnumBooleanMember {
        id: B,
        init: B,
    },
    EnumNumberMember {
        id: B,
        init: B,
    },
    EnumStringMember {
        id: B,
        init: B,
    },
    EnumBigIntMember {
        id: B,
        init: B,
    },

    // ─────────────────────────────── JSX ───────────────────────────────
    JSXElement {
        opening_element: B,
        closing_element: Opt,
        children: Vec<Node>,
    },
    JSXFragment {
        opening_fragment: B,
        closing_fragment: B,
        children: Vec<Node>,
    },
    JSXOpeningElement {
        name: B,
        attributes: Vec<Node>,
        self_closing: bool,
        type_arguments: Opt,
    },
    JSXClosingElement {
        name: B,
    },
    JSXOpeningFragment,
    JSXClosingFragment,
    JSXAttribute {
        name: B,
        value: Opt,
    },
    JSXSpreadAttribute {
        argument: B,
    },
    JSXIdentifier {
        name: String,
    },
    JSXNamespacedName {
        namespace: B,
        name: B,
    },
    JSXMemberExpression {
        object: B,
        property: B,
    },
    JSXText {
        value: String,
        raw: String,
    },
    JSXExpressionContainer {
        expression: B,
    },
    JSXSpreadChild {
        expression: B,
    },
    JSXEmptyExpression,
}

impl Kind {
    /// A string / number / boolean / null literal (type inferred from `value`).
    pub fn lit(value: Value, raw: String) -> Kind {
        let literal_type = match &value {
            Value::String(_) => "string",
            Value::Number(_) => "numeric",
            Value::Bool(_) => "boolean",
            _ => "null",
        };
        Kind::Literal { value, raw, literal_type, regex: None, bigint: None }
    }

    /// A numeric literal; `literalType` stays "numeric" even for non-finite values.
    pub fn num_lit(value: Value, raw: String) -> Kind {
        Kind::Literal { value, raw, literal_type: "numeric", regex: None, bigint: None }
    }

    pub fn str_lit(s: String, raw: String) -> Kind {
        Kind::lit(Value::String(s), raw)
    }

    pub fn regex_lit(pattern: String, flags: String, raw: String) -> Kind {
        Kind::Literal {
            value: Value::Null,
            raw,
            literal_type: "regexp",
            regex: Some(serde_json::json!({ "pattern": pattern, "flags": flags })),
            bigint: None,
        }
    }

    /// `digits` is the literal without the trailing `n` (may be hex/octal/binary).
    pub fn bigint_lit(digits: &str, raw: String) -> Kind {
        Kind::Literal {
            value: Value::Null,
            raw,
            literal_type: "bigint",
            regex: None,
            bigint: Some(bigint_to_decimal(digits)),
        }
    }
}

/// Convert a BigInt literal body (`0x1F`, `0b11`, `0o7`, `1_000`) to decimal.
pub fn bigint_to_decimal(digits: &str) -> String {
    let d: String = digits.chars().filter(|c| *c != '_').collect();
    let (radix, body) = match d.get(..2).map(|p| p.to_ascii_lowercase()) {
        Some(ref p) if p == "0x" => (16, &d[2..]),
        Some(ref p) if p == "0o" => (8, &d[2..]),
        Some(ref p) if p == "0b" => (2, &d[2..]),
        _ => (10, &d[..]),
    };
    if radix == 10 {
        let t = body.trim_start_matches('0');
        return if t.is_empty() { "0".into() } else { t.to_string() };
    }
    // Little-endian base 1e9 accumulator.
    let mut limbs: Vec<u64> = vec![0];
    for c in body.chars() {
        let mut carry = c.to_digit(radix).unwrap_or(0) as u64;
        for limb in limbs.iter_mut() {
            let v = *limb * radix as u64 + carry;
            *limb = v % 1_000_000_000;
            carry = v / 1_000_000_000;
        }
        if carry > 0 {
            limbs.push(carry);
        }
    }
    let mut out = limbs.last().unwrap().to_string();
    for l in limbs.iter().rev().skip(1) {
        out.push_str(&format!("{:09}", l));
    }
    out
}
