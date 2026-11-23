// @flow
type A<T: {x: number} = {}> = {| +a: T, -b?: ?string, [k: string]: mixed |};
opaque type O: string = string;
declare export function f<T>(x: T, ...rest: Array<T>): T %checks(x);
export type { A };
class C<+T> extends D<T> implements I { static +p: number = 1; #q: T; m<U>(x: U): this {} }
const g = async <T>(x: T): Promise<T> => (x: any);
enum E of string { A = 'a', B = 'b' }
