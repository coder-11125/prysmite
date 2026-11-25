//! A small MessagePack encoder for the serialized tree (`--format msgpack`): about half the size of
//! the JSON and cheaper to decode in other languages.

use serde_json::Value;

pub fn to_vec(v: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    write(v, &mut out);
    out
}

fn len_header(out: &mut Vec<u8>, n: usize, fix: (u8, usize), b16: u8, b32: u8) {
    if n < fix.1 {
        out.push(fix.0 | n as u8);
    } else if n <= u16::MAX as usize {
        out.push(b16);
        out.extend((n as u16).to_be_bytes());
    } else {
        out.push(b32);
        out.extend((n as u32).to_be_bytes());
    }
}

fn write(v: &Value, out: &mut Vec<u8>) {
    match v {
        Value::Null => out.push(0xc0),
        Value::Bool(false) => out.push(0xc2),
        Value::Bool(true) => out.push(0xc3),
        Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                match u {
                    0..=0x7f => out.push(u as u8),
                    0x80..=0xff => out.extend([0xcc, u as u8]),
                    0x100..=0xffff => {
                        out.push(0xcd);
                        out.extend((u as u16).to_be_bytes());
                    }
                    0x1_0000..=0xffff_ffff => {
                        out.push(0xce);
                        out.extend((u as u32).to_be_bytes());
                    }
                    _ => {
                        out.push(0xcf);
                        out.extend(u.to_be_bytes());
                    }
                }
            } else if let Some(i) = n.as_i64() {
                match i {
                    -32..=-1 => out.push(i as i8 as u8),
                    -128..=-33 => out.extend([0xd0, i as i8 as u8]),
                    -32768..=-129 => {
                        out.push(0xd1);
                        out.extend((i as i16).to_be_bytes());
                    }
                    -2_147_483_648..=-32769 => {
                        out.push(0xd2);
                        out.extend((i as i32).to_be_bytes());
                    }
                    _ => {
                        out.push(0xd3);
                        out.extend(i.to_be_bytes());
                    }
                }
            } else {
                out.push(0xcb);
                out.extend(n.as_f64().unwrap_or(f64::NAN).to_be_bytes());
            }
        }
        Value::String(s) => {
            let n = s.len();
            if n < 32 {
                out.push(0xa0 | n as u8);
            } else if n <= 0xff {
                out.extend([0xd9, n as u8]);
            } else if n <= 0xffff {
                out.push(0xda);
                out.extend((n as u16).to_be_bytes());
            } else {
                out.push(0xdb);
                out.extend((n as u32).to_be_bytes());
            }
            out.extend(s.as_bytes());
        }
        Value::Array(a) => {
            len_header(out, a.len(), (0x90, 16), 0xdc, 0xdd);
            a.iter().for_each(|x| write(x, out));
        }
        Value::Object(m) => {
            len_header(out, m.len(), (0x80, 16), 0xde, 0xdf);
            for (k, x) in m {
                write(&Value::String(k.clone()), out);
                write(x, out);
            }
        }
    }
}
