component Foo(a: number, 'data-id' as id: string = 'x', ...rest: Props) renders Bar { return <div {...rest}>{a}</div>; }
hook useThing(x: number): number { return x; }
const r = match (x) { 1 => 'a', [const a, ...] => a, {foo: 'b', ...} if (c) => 1, _ => 0 };
record Point { x: number, y: number = 0, static origin(): Point { return Point {x: 0}; } }
type K = keyof O; type T = $ReadOnly<{[K in keyof O]: O[K]}>; type X = T extends infer U ? U : empty;
declare component Baz(x: number) renders? Qux;
