//! Buffer members from table rows B1-B3 (variable-width ints, BigInt64, string slices and writes, subarray,
//! inspect, parent, offset). Each probe is the JS run unchanged under Node 22 (v22.23.1); its stdout is inlined
//! as the expected text. No Node needed at test time.
#[cfg(test)]
use super::*;

fn run_probe(src: &str, expected: &str) {
    let mut it = Interp::new();
    it.run(src).expect("probe runs");
    assert_eq!(it.out.join("\n"), expected);
}

// B1: readUIntLE/BE, readIntLE/BE, writeUIntLE/BE, writeIntLE/BE (byteLength 1..6), Uint aliases. Values, ranges, errors.
const BUFFER_VARWIDTH_INTS_MATCH_NODE_JS: &str = r#"
function E(f) { try { var r = f(); return 'ok:' + (typeof r === 'string' ? JSON.stringify(r) : String(r)); } catch (e) { return e.constructor.name + '|' + e.code + '|' + e.message; } }
var out = [];
function P(l, f) { out.push(l + ' => ' + E(f)); }
var b = Buffer.from([1,2,3,4,5,6,7,0x80]);
P('UIntLE(0,6)', () => b.readUIntLE(0, 6));
P('UIntBE(0,6)', () => b.readUIntBE(0, 6));
P('IntLE(0,6)', () => b.readIntLE(0, 6));
P('IntBE(0,6)', () => b.readIntBE(0, 6));
P('IntLE(0,1)', () => b.readIntLE(0, 1));
P('IntBE(5,3) 0x80', () => b.readIntBE(5, 3));
P('bl0', () => b.readUIntLE(0, 0));
P('bl7', () => b.readUIntLE(0, 7));
P('bl undef', () => b.readUIntLE(0));
P('bl str', () => b.readUIntLE(0, '2'));
P('bl 2.5', () => b.readUIntLE(0, 2.5));
P('off undef', () => b.readUIntLE(undefined, 2));
P('off -1', () => b.readUIntLE(-1, 2));
P('off 7 bl2', () => b.readUIntLE(7, 2));
P('off 6 bl2', () => b.readUIntLE(6, 2));
P('off 1.5', () => b.readUIntLE(1.5, 2));
P('off str', () => b.readUIntLE('1', 2));
P('off NaN', () => b.readUIntLE(NaN, 2));
P('off 6 bl3 len8', () => Buffer.alloc(8).readIntLE(6, 3));
P('small buf', () => Buffer.alloc(2).readUIntLE(0, 3));
P('empty buf', () => Buffer.alloc(0).readIntLE(0, 1));
P('rIntBE(0,6) 0x7f', () => Buffer.from([0x7f,0xff,0xff,0xff,0xff,0xff]).readIntBE(0,6));
P('rUIntBE(0,6) 0xff', () => Buffer.from([0xff,0xff,0xff,0xff,0xff,0xff]).readUIntBE(0,6));
P('rIntLE(0,6) 0xff', () => Buffer.from([0xff,0xff,0xff,0xff,0xff,0xff]).readIntLE(0,6));
P('rIntLE(0,6) 0x7f top', () => Buffer.from([0xff,0xff,0xff,0xff,0xff,0x7f]).readIntLE(0,6));
P('rUIntLE 1f', () => Buffer.from([0xff,0xff,0xff,0xff,0xff,0x1f]).readUIntLE(0,6));
P('rUIntBE 6 mixed', () => Buffer.from([0x12,0x34,0x56,0x78,0x9a,0xbc]).readUIntBE(0,6));
P('rIntLE 1 neg', () => Buffer.from([0x80]).readIntLE(0,1));
P('rIntBE 3 sign', () => Buffer.from([0xff,0xff,0x7f]).readIntLE(0,3));
P('rIntBE 4 min', () => Buffer.from([0x80,0,0,0]).readIntBE(0,4));
var w = Buffer.alloc(8);
P('wUIntLE 3', () => w.writeUIntLE(0x123456, 0, 3) + ':' + w.toString('hex'));
P('wUIntBE 6 max', () => w.writeUIntBE(2**48-1, 0, 6) + ':' + w.toString('hex'));
P('wUIntBE 6 over', () => w.writeUIntBE(2**48, 0, 6));
P('wUIntBE neg', () => w.writeUIntBE(-1, 0, 2));
P('wIntLE 3 min', () => w.writeIntLE(-8388608, 0, 3) + ':' + w.toString('hex'));
P('wIntLE 3 under', () => w.writeIntLE(-8388609, 0, 3));
P('wIntLE 3 over', () => w.writeIntLE(8388608, 0, 3));
P('wIntBE 6 -1', () => w.writeIntBE(-1, 0, 6) + ':' + w.toString('hex'));
P('wIntBE 6 -2^47', () => w.writeIntBE(-(2**47), 0, 6) + ':' + w.toString('hex'));
P('wIntBE 6 under', () => w.writeIntBE(-(2**47)-1, 0, 6));
P('wIntBE 6 over', () => w.writeIntBE(2**47, 0, 6));
P('wIntBE 6 2^47-1', () => w.writeIntBE(2**47-1, 0, 6) + ':' + w.toString('hex'));
P('wUIntLE str value', () => w.writeUIntLE('5', 0, 2) + ':' + w.toString('hex'));
P('wUIntLE float', () => w.writeUIntLE(1.5, 0, 2) + ':' + w.toString('hex'));
P('wIntLE frac neg', () => w.writeIntLE(-1.5, 0, 2) + ':' + w.toString('hex'));
P('wUIntLE NaN', () => w.writeUIntLE(NaN, 0, 2) + ':' + w.toString('hex'));
P('wUIntLE null', () => w.writeUIntLE(null, 0, 2) + ':' + w.toString('hex'));
P('wUIntLE off 7', () => w.writeUIntLE(1, 7, 2));
P('wUIntLE off 6', () => w.writeUIntLE(1, 6, 2) + ':' + w.toString('hex'));
P('wUIntLE off undef', () => w.writeUIntLE(1, undefined, 2));
P('wUIntLE bl undef', () => w.writeUIntLE(1, 0));
P('wIntLE bl 7', () => w.writeIntLE(1, 0, 7));
P('wUIntLE bl 0', () => w.writeUIntLE(1, 0, 0));
P('wUIntLE ret', () => w.writeUIntLE(1, 2, 1));
P('wUIntLE 2^53', () => w.writeUIntLE(2**53, 0, 6));
P('wIntLE 4 unsigned over', () => w.writeUIntBE(2**32, 0, 4));
P('wIntBE 4 under', () => w.writeIntBE(-(2**31)-1, 0, 4));
P('wIntBE 4 ok', () => w.writeIntBE(2**31-1, 0, 4));
P('wUIntBE 5 over', () => w.writeUIntBE(2**40, 0, 5));
P('wUIntBE 3 over', () => w.writeUIntBE(2**24, 0, 3));
P('wIntLE 5 under', () => w.writeIntLE(-(2**39)-1, 0, 5));
P('wIntLE value undef', () => w.writeIntLE(undefined, 0, 2) + ':' + w.toString('hex'));
P('wUIntLE small buf', () => Buffer.alloc(1).writeUIntLE(1, 0, 2));
var a = Buffer.prototype;
P('alias identity', () => [a.readUintBE === a.readUIntBE, a.readUintLE === a.readUIntLE, a.writeUintBE === a.writeUIntBE, a.writeUintLE === a.writeUIntLE].join(','));
P('alias names', () => [a.readUintBE.name, a.writeUintLE.name, a.readUintLE.length, a.writeUintBE.length].join(','));
P('canon name/len', () => [a.readIntLE.name, a.readIntLE.length, a.writeIntBE.name, a.writeIntBE.length, a.writeUIntBE.length, a.readUIntBE.name, a.readIntBE.name].join(','));
P('canon proto', () => [a.readIntBE.hasOwnProperty('prototype'), a.writeIntBE.hasOwnProperty('prototype')].join(','));
console.log(out.join('\n'));
"#;
const BUFFER_VARWIDTH_INTS_MATCH_NODE_OUT: &str = r#"UIntLE(0,6) => ok:6618611909121
UIntBE(0,6) => ok:1108152157446
IntLE(0,6) => ok:6618611909121
IntBE(0,6) => ok:1108152157446
IntLE(0,1) => ok:1
IntBE(5,3) 0x80 => ok:395136
bl0 => RangeError|ERR_OUT_OF_RANGE|The value of "byteLength" is out of range. It must be >= 1 and <= 6. Received 0
bl7 => RangeError|ERR_OUT_OF_RANGE|The value of "byteLength" is out of range. It must be >= 1 and <= 6. Received 7
bl undef => TypeError|ERR_INVALID_ARG_TYPE|The "byteLength" argument must be of type number. Received undefined
bl str => TypeError|ERR_INVALID_ARG_TYPE|The "byteLength" argument must be of type number. Received type string ('2')
bl 2.5 => RangeError|ERR_OUT_OF_RANGE|The value of "byteLength" is out of range. It must be an integer. Received 2.5
off undef => TypeError|ERR_INVALID_ARG_TYPE|The "offset" argument must be of type number. Received undefined
off -1 => RangeError|ERR_OUT_OF_RANGE|The value of "offset" is out of range. It must be >= 0 and <= 6. Received -1
off 7 bl2 => RangeError|ERR_OUT_OF_RANGE|The value of "offset" is out of range. It must be >= 0 and <= 6. Received 7
off 6 bl2 => ok:32775
off 1.5 => RangeError|ERR_OUT_OF_RANGE|The value of "offset" is out of range. It must be an integer. Received 1.5
off str => TypeError|ERR_INVALID_ARG_TYPE|The "offset" argument must be of type number. Received type string ('1')
off NaN => RangeError|ERR_OUT_OF_RANGE|The value of "offset" is out of range. It must be an integer. Received NaN
off 6 bl3 len8 => RangeError|ERR_OUT_OF_RANGE|The value of "offset" is out of range. It must be >= 0 and <= 5. Received 6
small buf => RangeError|ERR_BUFFER_OUT_OF_BOUNDS|Attempt to access memory outside buffer bounds
empty buf => RangeError|ERR_BUFFER_OUT_OF_BOUNDS|Attempt to access memory outside buffer bounds
rIntBE(0,6) 0x7f => ok:140737488355327
rUIntBE(0,6) 0xff => ok:281474976710655
rIntLE(0,6) 0xff => ok:-1
rIntLE(0,6) 0x7f top => ok:140737488355327
rUIntLE 1f => ok:35184372088831
rUIntBE 6 mixed => ok:20015998343868
rIntLE 1 neg => ok:-128
rIntBE 3 sign => ok:8388607
rIntBE 4 min => ok:-2147483648
wUIntLE 3 => ok:"3:5634120000000000"
wUIntBE 6 max => ok:"6:ffffffffffff0000"
wUIntBE 6 over => RangeError|ERR_OUT_OF_RANGE|The value of "value" is out of range. It must be >= 0 and < 2 ** 48. Received 281_474_976_710_656
wUIntBE neg => RangeError|ERR_OUT_OF_RANGE|The value of "value" is out of range. It must be >= 0 and <= 65535. Received -1
wIntLE 3 min => ok:"3:000080ffffff0000"
wIntLE 3 under => RangeError|ERR_OUT_OF_RANGE|The value of "value" is out of range. It must be >= -8388608 and <= 8388607. Received -8388609
wIntLE 3 over => RangeError|ERR_OUT_OF_RANGE|The value of "value" is out of range. It must be >= -8388608 and <= 8388607. Received 8388608
wIntBE 6 -1 => ok:"6:ffffffffffff0000"
wIntBE 6 -2^47 => ok:"6:8000000000000000"
wIntBE 6 under => RangeError|ERR_OUT_OF_RANGE|The value of "value" is out of range. It must be >= -(2 ** 47) and < 2 ** 47. Received -140_737_488_355_329
wIntBE 6 over => RangeError|ERR_OUT_OF_RANGE|The value of "value" is out of range. It must be >= -(2 ** 47) and < 2 ** 47. Received 140_737_488_355_328
wIntBE 6 2^47-1 => ok:"6:7fffffffffff0000"
wUIntLE str value => ok:"2:0500ffffffff0000"
wUIntLE float => ok:"2:0100ffffffff0000"
wIntLE frac neg => ok:"2:ffffffffffff0000"
wUIntLE NaN => ok:"2:0000ffffffff0000"
wUIntLE null => ok:"2:0000ffffffff0000"
wUIntLE off 7 => RangeError|ERR_OUT_OF_RANGE|The value of "offset" is out of range. It must be >= 0 and <= 6. Received 7
wUIntLE off 6 => ok:"8:0000ffffffff0100"
wUIntLE off undef => TypeError|ERR_INVALID_ARG_TYPE|The "offset" argument must be of type number. Received undefined
wUIntLE bl undef => TypeError|ERR_INVALID_ARG_TYPE|The "byteLength" argument must be of type number. Received undefined
wIntLE bl 7 => RangeError|ERR_OUT_OF_RANGE|The value of "byteLength" is out of range. It must be >= 1 and <= 6. Received 7
wUIntLE bl 0 => RangeError|ERR_OUT_OF_RANGE|The value of "byteLength" is out of range. It must be >= 1 and <= 6. Received 0
wUIntLE ret => ok:3
wUIntLE 2^53 => RangeError|ERR_OUT_OF_RANGE|The value of "value" is out of range. It must be >= 0 and < 2 ** 48. Received 9_007_199_254_740_992
wIntLE 4 unsigned over => RangeError|ERR_OUT_OF_RANGE|The value of "value" is out of range. It must be >= 0 and <= 4294967295. Received 4294967296
wIntBE 4 under => RangeError|ERR_OUT_OF_RANGE|The value of "value" is out of range. It must be >= -2147483648 and <= 2147483647. Received -2147483649
wIntBE 4 ok => ok:4
wUIntBE 5 over => RangeError|ERR_OUT_OF_RANGE|The value of "value" is out of range. It must be >= 0 and < 2 ** 40. Received 1_099_511_627_776
wUIntBE 3 over => RangeError|ERR_OUT_OF_RANGE|The value of "value" is out of range. It must be >= 0 and <= 16777215. Received 16777216
wIntLE 5 under => RangeError|ERR_OUT_OF_RANGE|The value of "value" is out of range. It must be >= -(2 ** 39) and < 2 ** 39. Received -549_755_813_889
wIntLE value undef => ok:"2:0000ffffffff0100"
wUIntLE small buf => RangeError|ERR_BUFFER_OUT_OF_BOUNDS|Attempt to access memory outside buffer bounds
alias identity => ok:"true,true,true,true"
alias names => ok:"readUIntBE,writeUIntLE,2,3"
canon name/len => ok:"readIntLE,2,writeIntBE,3,3,readUIntBE,readIntBE"
canon proto => ok:"true,true""#;
#[test]
fn buffer_varwidth_ints_match_node() {
    run_probe(
        BUFFER_VARWIDTH_INTS_MATCH_NODE_JS,
        BUFFER_VARWIDTH_INTS_MATCH_NODE_OUT,
    );
}

// B2: readBigInt64/readBigUInt64 and write variants (LE/BE), BigInt range errors, Uint aliases.
const BUFFER_BIGINT64_MATCH_NODE_JS: &str = r#"
function E(f) { try { var r = f(); return 'ok:' + (typeof r === 'string' ? JSON.stringify(r) : String(r)); } catch (e) { return e.constructor.name + '|' + e.code + '|' + e.message; } }
var out = [];
function P(l, f) { out.push(l + ' => ' + E(f)); }
var b = Buffer.from([1,2,3,4,5,6,7,0x80,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff]);
P('rBI64LE 0', () => b.readBigInt64LE(0));
P('rBI64BE 0', () => b.readBigInt64BE(0));
P('rBI64LE 8', () => b.readBigInt64LE(8));
P('rBUI64LE 8', () => b.readBigUInt64LE(8));
P('rBUI64LE default', () => b.readBigUInt64LE());
P('rBI64LE off 1', () => b.readBigInt64LE(1));
P('rBI64LE off 9', () => b.readBigInt64LE(9));
P('rBI64LE off -1', () => b.readBigInt64LE(-1));
P('rBI64LE off null', () => b.readBigInt64LE(null));
P('rBI64LE off 1.5', () => b.readBigInt64LE(1.5));
P('rBI64LE short', () => Buffer.alloc(4).readBigInt64LE(0));
P('rBI64LE short default', () => Buffer.alloc(4).readBigUInt64LE());
P('rBUI64LE str', () => b.readBigUInt64LE('0'));
P('rBUI64BE', () => Buffer.from([0x12,0x34,0x56,0x78,0x9a,0xbc,0xde,0xf0]).readBigUInt64BE(0));
P('rBI64BE min', () => Buffer.from([0x80,0,0,0,0,0,0,0]).readBigInt64BE(0));
P('rBI64BE max', () => Buffer.from([0x7f,0xff,0xff,0xff,0xff,0xff,0xff,0xff]).readBigInt64BE(0));
var w = Buffer.alloc(16);
P('wBI64LE -2', () => w.writeBigInt64LE(-2n, 0) + ':' + w.toString('hex'));
P('wBI64BE max', () => w.writeBigInt64BE(2n**63n-1n, 0) + ':' + w.toString('hex'));
P('wBUI64LE max', () => w.writeBigUInt64LE(2n**64n-1n, 8) + ':' + w.toString('hex'));
P('wBUI64BE 0', () => w.writeBigUInt64BE(0n, 0) + ':' + w.toString('hex'));
P('wBUI64 over', () => w.writeBigUInt64LE(2n**64n, 0));
P('wBUI64 neg', () => w.writeBigUInt64LE(-1n, 0));
P('wBI64 over', () => w.writeBigInt64LE(2n**63n, 0));
P('wBI64 under', () => w.writeBigInt64LE(-(2n**63n)-1n, 0));
P('wBI64 min ok', () => w.writeBigInt64LE(-(2n**63n), 0) + ':' + w.toString('hex'));
P('wBI64 number', () => w.writeBigInt64LE(5, 0));
P('wBI64 string', () => w.writeBigInt64LE('5', 0));
P('wBUI64 number', () => w.writeBigUInt64LE(5, 0));
P('wBUI64 undefined', () => w.writeBigUInt64LE(undefined, 0));
P('wBUI64 default off', () => w.writeBigUInt64LE(7n) + ':' + w.toString('hex'));
P('wBI64 off 9', () => w.writeBigInt64LE(1n, 9));
P('wBI64 off 8', () => w.writeBigInt64LE(1n, 8));
P('wBI64 off 1.5', () => w.writeBigInt64LE(1n, 1.5));
P('wBI64 BE ret', () => w.writeBigInt64BE(1n, 4));
P('wBI64 small', () => Buffer.alloc(4).writeBigInt64BE(1n, 0));
P('wBUI64 fraction', () => w.writeBigUInt64LE(1.5, 0));
P('wBUI64 -5n msg', () => w.writeBigUInt64LE(-5n, 0));
P('wBI64 2^70 msg', () => w.writeBigInt64LE(2n**70n, 0));
P('wBI64 -2^70 msg', () => w.writeBigInt64BE(-(2n**70n), 0));
P('wBUI64 2^70 msg', () => w.writeBigUInt64LE(2n**70n, 0));
P('wBI64 off null', () => w.writeBigInt64LE(1n, null));
P('wBUI64 ret', () => w.writeBigUInt64LE(3n, 8));
P('wBI64 default ret', () => w.writeBigInt64LE(3n));
var a = Buffer.prototype;
P('aliases', () => [a.readBigUint64LE === a.readBigUInt64LE, a.readBigUint64BE === a.readBigUInt64BE, a.writeBigUint64LE === a.writeBigUInt64LE, a.writeBigUint64BE === a.writeBigUInt64BE].join(','));
P('names', () => [a.readBigUint64BE.name, a.writeBigUint64LE.name, a.readBigInt64LE.name, a.readBigUInt64LE.length, a.readBigInt64BE.length, a.writeBigInt64LE.length, a.writeBigUInt64BE.length, a.writeBigUint64LE.length].join(','));
P('protos', () => [a.readBigInt64LE.hasOwnProperty('prototype'), a.writeBigUInt64BE.hasOwnProperty('prototype')].join(','));
console.log(out.join('\n'));
"#;
const BUFFER_BIGINT64_MATCH_NODE_OUT: &str = r#"rBI64LE 0 => ok:-9221395093405892095
rBI64BE 0 => ok:72623859790382976
rBI64LE 8 => ok:-1
rBUI64LE 8 => ok:18446744073709551615
rBUI64LE default => ok:9225348980303659521
rBI64LE off 1 => ok:-36021074583616766
rBI64LE off 9 => RangeError|ERR_OUT_OF_RANGE|The value of "offset" is out of range. It must be >= 0 and <= 8. Received 9
rBI64LE off -1 => RangeError|ERR_OUT_OF_RANGE|The value of "offset" is out of range. It must be >= 0 and <= 8. Received -1
rBI64LE off null => TypeError|ERR_INVALID_ARG_TYPE|The "offset" argument must be of type number. Received null
rBI64LE off 1.5 => RangeError|ERR_OUT_OF_RANGE|The value of "offset" is out of range. It must be an integer. Received 1.5
rBI64LE short => RangeError|ERR_BUFFER_OUT_OF_BOUNDS|Attempt to access memory outside buffer bounds
rBI64LE short default => RangeError|ERR_BUFFER_OUT_OF_BOUNDS|Attempt to access memory outside buffer bounds
rBUI64LE str => TypeError|ERR_INVALID_ARG_TYPE|The "offset" argument must be of type number. Received type string ('0')
rBUI64BE => ok:1311768467463790320
rBI64BE min => ok:-9223372036854775808
rBI64BE max => ok:9223372036854775807
wBI64LE -2 => ok:"8:feffffffffffffff0000000000000000"
wBI64BE max => ok:"8:7fffffffffffffff0000000000000000"
wBUI64LE max => ok:"16:7fffffffffffffffffffffffffffffff"
wBUI64BE 0 => ok:"8:0000000000000000ffffffffffffffff"
wBUI64 over => RangeError|ERR_OUT_OF_RANGE|The value of "value" is out of range. It must be >= 0n and < 2n ** 64n. Received 18_446_744_073_709_551_616n
wBUI64 neg => RangeError|ERR_OUT_OF_RANGE|The value of "value" is out of range. It must be >= 0n and < 2n ** 64n. Received -1n
wBI64 over => RangeError|ERR_OUT_OF_RANGE|The value of "value" is out of range. It must be >= -(2n ** 63n) and < 2n ** 63n. Received 9_223_372_036_854_775_808n
wBI64 under => RangeError|ERR_OUT_OF_RANGE|The value of "value" is out of range. It must be >= -(2n ** 63n) and < 2n ** 63n. Received -9_223_372_036_854_775_809n
wBI64 min ok => ok:"8:0000000000000080ffffffffffffffff"
wBI64 number => TypeError|undefined|Cannot mix BigInt and other types, use explicit conversions
wBI64 string => TypeError|undefined|Cannot mix BigInt and other types, use explicit conversions
wBUI64 number => TypeError|undefined|Cannot mix BigInt and other types, use explicit conversions
wBUI64 undefined => TypeError|undefined|Cannot mix BigInt and other types, use explicit conversions
wBUI64 default off => ok:"8:0700000000000000ffffffffffffffff"
wBI64 off 9 => RangeError|ERR_OUT_OF_RANGE|The value of "offset" is out of range. It must be >= 0 and <= 8. Received 9
wBI64 off 8 => ok:16
wBI64 off 1.5 => RangeError|ERR_OUT_OF_RANGE|The value of "offset" is out of range. It must be an integer. Received 1.5
wBI64 BE ret => ok:12
wBI64 small => RangeError|ERR_BUFFER_OUT_OF_BOUNDS|Attempt to access memory outside buffer bounds
wBUI64 fraction => TypeError|undefined|Cannot mix BigInt and other types, use explicit conversions
wBUI64 -5n msg => RangeError|ERR_OUT_OF_RANGE|The value of "value" is out of range. It must be >= 0n and < 2n ** 64n. Received -5n
wBI64 2^70 msg => RangeError|ERR_OUT_OF_RANGE|The value of "value" is out of range. It must be >= -(2n ** 63n) and < 2n ** 63n. Received 1_180_591_620_717_411_303_424n
wBI64 -2^70 msg => RangeError|ERR_OUT_OF_RANGE|The value of "value" is out of range. It must be >= -(2n ** 63n) and < 2n ** 63n. Received -1_180_591_620_717_411_303_424n
wBUI64 2^70 msg => RangeError|ERR_OUT_OF_RANGE|The value of "value" is out of range. It must be >= 0n and < 2n ** 64n. Received 1_180_591_620_717_411_303_424n
wBI64 off null => TypeError|ERR_INVALID_ARG_TYPE|The "offset" argument must be of type number. Received null
wBUI64 ret => ok:16
wBI64 default ret => ok:8
aliases => ok:"true,true,true,true"
names => ok:"readBigUInt64BE,writeBigUInt64LE,readBigInt64LE,0,0,1,1,1"
protos => ok:"true,true""#;
#[test]
fn buffer_bigint64_match_node() {
    run_probe(
        BUFFER_BIGINT64_MATCH_NODE_JS,
        BUFFER_BIGINT64_MATCH_NODE_OUT,
    );
}

// B3a: utf8/hex/base64/base64url/ascii/latin1/ucs2 Slice and Write, offset/length rules, no split of UTF-8 code points.
const BUFFER_STRING_SLICES_AND_WRITES_MATCH_NODE_JS: &str = r#"
function E(f) { try { var r = f(); return 'ok:' + (typeof r === 'string' ? JSON.stringify(r) : String(r)); } catch (e) { return e.constructor.name + '|' + e.code + '|' + e.message; } }
var out = [];
function P(l, f) { out.push(l + ' => ' + E(f)); }
var s = 'hé€😀z';
var b = Buffer.from(s, 'utf8');
P('len', () => b.length);
P('utf8Slice 0 3', () => b.utf8Slice(0, 3));
P('utf8Slice 1 3', () => b.utf8Slice(1, 3));
P('utf8Slice undef', () => b.utf8Slice());
P('utf8Slice 4', () => b.utf8Slice(4));
P('utf8Slice neg', () => b.utf8Slice(-3, 100));
P('utf8Slice 0 100', () => b.utf8Slice(0, 100));
P('utf8Slice 20', () => b.utf8Slice(20));
P('utf8Slice 5 2', () => b.utf8Slice(5, 2));
P('utf8Slice 1.5 3', () => b.utf8Slice(1.5, 3));
P('utf8Slice NaN 3', () => b.utf8Slice(NaN, 3));
P('utf8Slice str end', () => b.utf8Slice(0, '2'));
P('utf8Slice invalid', () => Buffer.from([0xff, 0x41, 0xe2, 0x82]).utf8Slice());
P('hexSlice', () => b.hexSlice(0, 4));
P('hexSlice 0 0', () => b.hexSlice(0, 0));
P('hexSlice all', () => b.hexSlice());
P('hexSlice 0 100', () => b.hexSlice(0, 100));
P('base64Slice', () => b.base64Slice(0, 5));
P('base64urlSlice', () => Buffer.from([0xfb, 0xff, 0xfe]).base64urlSlice());
P('base64Slice pad', () => Buffer.from([0xfb, 0xff]).base64Slice());
P('asciiSlice', () => Buffer.from([0x41, 0xff, 0x80, 0x7f]).asciiSlice());
P('latin1Slice', () => Buffer.from([0x41, 0xff, 0x80, 0x7f]).latin1Slice());
P('ucs2Slice', () => Buffer.from([0x41, 0x00, 0x42, 0x00, 0x43]).ucs2Slice());
P('ucs2Slice 1 3', () => Buffer.from([0x41, 0x00, 0x42, 0x00, 0x43]).ucs2Slice(1, 3));
P('ucs2Slice odd', () => Buffer.from([0x41,0x00,0x42]).ucs2Slice());
P('slice lengths', () => [Buffer.prototype.utf8Slice.length, Buffer.prototype.hexSlice.length, Buffer.prototype.base64urlSlice.length, Buffer.prototype.ucs2Slice.length, Buffer.prototype.utf8Slice.name, Buffer.prototype.asciiSlice.name].join(','));
var w = Buffer.alloc(10);
P('utf8Write', () => w.utf8Write('hé€', 0) + ' ' + w.toString('hex'));
var w2 = Buffer.alloc(4);
P('utf8Write split', () => w2.utf8Write('a€', 0) + ' ' + w2.toString('hex'));
var w3 = Buffer.alloc(4);
P('utf8Write len cut', () => w3.utf8Write('abc€', 1, 3) + ' ' + w3.toString('hex'));
var w4 = Buffer.alloc(6);
P('utf8Write 4byte', () => w4.utf8Write('😀😀', 0, 5) + ' ' + w4.toString('hex'));
P('utf8Write off 6', () => w4.utf8Write('x', 6));
P('utf8Write off 7', () => w4.utf8Write('x', 7));
P('utf8Write off -1', () => w4.utf8Write('x', -1));
P('utf8Write off NaN', () => w4.utf8Write('x', NaN) + ' ' + w4.toString('hex'));
P('utf8Write len 0', () => w4.utf8Write('xyz', 0, 0));
P('utf8Write len -1', () => w4.utf8Write('xyz', 0, -1));
P('utf8Write len big', () => w4.utf8Write('xyz', 1, 100) + ' ' + w4.toString('hex'));
P('utf8Write num', () => w4.utf8Write(12, 0));
P('utf8Write undef', () => w4.utf8Write(undefined, 0));
var w5 = Buffer.alloc(4);
P('hexWrite', () => w5.hexWrite('0a0b0c', 0) + ' ' + w5.toString('hex'));
P('hexWrite odd', () => w5.hexWrite('abc', 0) + ' ' + w5.toString('hex'));
P('hexWrite bad', () => w5.hexWrite('zz', 0) + ' ' + w5.toString('hex'));
P('hexWrite off 3', () => w5.hexWrite('0102', 3) + ' ' + w5.toString('hex'));
P('hexWrite len 1', () => w5.hexWrite('0102', 0, 1) + ' ' + w5.toString('hex'));
P('hexWrite len 0', () => w5.hexWrite('0102', 0, 0));
P('hexWrite off 1 len 2', () => w5.hexWrite('0102030405', 1, 2) + ' ' + w5.toString('hex'));
P('hexWrite off 1.5', () => w5.hexWrite('01', 1.5) + ' ' + w5.toString('hex'));
P('hexWrite off -1', () => w5.hexWrite('01', -1));
P('hexWrite off 5', () => w5.hexWrite('01', 5));
P('hexWrite len -1', () => w5.hexWrite('01', 0, -1));
P('hexWrite num', () => w5.hexWrite(5, 0));
P('base64Write', () => w5.base64Write('AQID', 0) + ' ' + w5.toString('hex'));
P('base64Write len 2', () => w5.base64Write('AQIDBA==', 0, 2) + ' ' + w5.toString('hex'));
P('base64urlWrite', () => w5.base64urlWrite('-_8', 0) + ' ' + w5.toString('hex'));
P('ucs2Write', () => w5.ucs2Write('ab', 0) + ' ' + w5.toString('hex'));
P('ucs2Write 3', () => w5.ucs2Write('abc', 0) + ' ' + w5.toString('hex'));
P('ucs2Write len 3', () => Buffer.alloc(8).ucs2Write('abc', 0, 3));
P('ucs2Write off -1', () => w5.ucs2Write('a', -1));
P('ucs2Write off 5', () => w5.ucs2Write('a', 5));
P('latin1Write', () => w5.latin1Write('ÿĀ', 0) + ' ' + w5.toString('hex'));
P('latin1Write off 2 len 2', () => { var x = Buffer.alloc(8); return x.latin1Write('abc', 2, 2) + ' ' + x.toString('hex'); });
P('asciiWrite', () => w5.asciiWrite('ÿA', 2) + ' ' + w5.toString('hex'));
P('latin1Write off -1', () => w5.latin1Write('x', -1));
P('latin1Write len -1', () => w5.latin1Write('x', 0, -1));
P('latin1Write off 4', () => w5.latin1Write('x', 4));
P('utf8Write off 4', () => w5.utf8Write('x', 4));
P('hexWrite len str', () => w5.hexWrite('0102', 0, '1') + ' ' + w5.toString('hex'));
P('write lengths', () => [Buffer.prototype.utf8Write.length, Buffer.prototype.hexWrite.length, Buffer.prototype.ucs2Write.length, Buffer.prototype.latin1Write.length, Buffer.prototype.base64urlWrite.length, Buffer.prototype.asciiWrite.length].join(','));
P('write protos', () => [Buffer.prototype.utf8Write.hasOwnProperty('prototype'), Buffer.prototype.hexWrite.hasOwnProperty('prototype'), Buffer.prototype.latin1Write.hasOwnProperty('prototype'), Buffer.prototype.asciiWrite.hasOwnProperty('prototype'), Buffer.prototype.utf8Slice.hasOwnProperty('prototype')].join(','));
console.log(out.join('\n'));
"#;
const BUFFER_STRING_SLICES_AND_WRITES_MATCH_NODE_OUT: &str = r#"len => ok:11
utf8Slice 0 3 => ok:"hé"
utf8Slice 1 3 => ok:"é"
utf8Slice undef => ok:"hé€😀z"
utf8Slice 4 => ok:"��😀z"
utf8Slice neg => RangeError|ERR_OUT_OF_RANGE|Index out of range
utf8Slice 0 100 => RangeError|ERR_OUT_OF_RANGE|Index out of range
utf8Slice 20 => RangeError|ERR_OUT_OF_RANGE|Index out of range
utf8Slice 5 2 => ok:""
utf8Slice 1.5 3 => ok:"é"
utf8Slice NaN 3 => ok:"hé"
utf8Slice str end => ok:"h�"
utf8Slice invalid => ok:"�A�"
hexSlice => ok:"68c3a9e2"
hexSlice 0 0 => ok:""
hexSlice all => ok:"68c3a9e282acf09f98807a"
hexSlice 0 100 => RangeError|ERR_OUT_OF_RANGE|Index out of range
base64Slice => ok:"aMOp4oI="
base64urlSlice => ok:"-__-"
base64Slice pad => ok:"+/8="
asciiSlice => ok:"A\u0000"
latin1Slice => ok:"Aÿ"
ucs2Slice => ok:"AB"
ucs2Slice 1 3 => ok:"䈀"
ucs2Slice odd => ok:"A"
slice lengths => ok:"0,0,0,0,utf8Slice,asciiSlice"
utf8Write => ok:"6 68c3a9e282ac00000000"
utf8Write split => ok:"4 61e282ac"
utf8Write len cut => ok:"3 00616263"
utf8Write 4byte => ok:"4 f09f98800000"
utf8Write off 6 => ok:0
utf8Write off 7 => RangeError|ERR_BUFFER_OUT_OF_BOUNDS|"offset" is outside of buffer bounds
utf8Write off -1 => RangeError|ERR_BUFFER_OUT_OF_BOUNDS|"offset" is outside of buffer bounds
utf8Write off NaN => ok:"1 789f98800000"
utf8Write len 0 => ok:0
utf8Write len -1 => RangeError|ERR_BUFFER_OUT_OF_BOUNDS|"length" is outside of buffer bounds
utf8Write len big => ok:"3 7878797a0000"
utf8Write num => TypeError|ERR_INVALID_ARG_TYPE|argument must be a string
utf8Write undef => TypeError|ERR_INVALID_ARG_TYPE|argument must be a string
hexWrite => ok:"3 0a0b0c00"
hexWrite odd => ok:"1 ab0b0c00"
hexWrite bad => ok:"0 ab0b0c00"
hexWrite off 3 => ok:"1 ab0b0c01"
hexWrite len 1 => ok:"1 010b0c01"
hexWrite len 0 => ok:0
hexWrite off 1 len 2 => ok:"2 01010201"
hexWrite off 1.5 => ok:"1 01010201"
hexWrite off -1 => RangeError|ERR_OUT_OF_RANGE|Index out of range
hexWrite off 5 => RangeError|ERR_BUFFER_OUT_OF_BOUNDS|"offset" is outside of buffer bounds
hexWrite len -1 => RangeError|ERR_OUT_OF_RANGE|Index out of range
hexWrite num => TypeError|ERR_INVALID_ARG_TYPE|argument must be a string
base64Write => ok:"3 01020301"
base64Write len 2 => ok:"2 01020301"
base64urlWrite => ok:"2 fbff0301"
ucs2Write => ok:"4 61006200"
ucs2Write 3 => ok:"4 61006200"
ucs2Write len 3 => ok:2
ucs2Write off -1 => RangeError|ERR_OUT_OF_RANGE|Index out of range
ucs2Write off 5 => RangeError|ERR_BUFFER_OUT_OF_BOUNDS|"offset" is outside of buffer bounds
latin1Write => ok:"2 ff006200"
latin1Write off 2 len 2 => ok:"2 0000616200000000"
asciiWrite => ok:"2 ff00ff41"
latin1Write off -1 => RangeError|ERR_BUFFER_OUT_OF_BOUNDS|"offset" is outside of buffer bounds
latin1Write len -1 => RangeError|ERR_BUFFER_OUT_OF_BOUNDS|"length" is outside of buffer bounds
latin1Write off 4 => ok:0
utf8Write off 4 => ok:0
hexWrite len str => ok:"1 0100ff41"
write lengths => ok:"1,0,0,1,0,1"
write protos => ok:"true,false,true,true,false""#;
#[test]
fn buffer_string_slices_and_writes_match_node() {
    run_probe(
        BUFFER_STRING_SLICES_AND_WRITES_MATCH_NODE_JS,
        BUFFER_STRING_SLICES_AND_WRITES_MATCH_NODE_OUT,
    );
}

// B3b: subarray (returns a Buffer), toLocaleString alias, inspect, parent and offset accessors.
const BUFFER_SUBARRAY_INSPECT_PARENT_OFFSET_MATCH_NODE_JS: &str = r#"
function E(f) { try { var r = f(); return 'ok:' + (typeof r === 'string' ? JSON.stringify(r) : String(r)); } catch (e) { return e.constructor.name + '|' + e.code + '|' + e.message; } }
var out = [];
function P(l, f) { out.push(l + ' => ' + E(f)); }
var sub = Buffer.from([1,2,3,4,5,6]);
P('subarray 1..4', () => { var v = sub.subarray(1, 4); return [v instanceof Buffer, Buffer.isBuffer(v), v.length, v[0], v.constructor.name, v.buffer === sub.buffer].join(','); });
P('subarray defaults', () => [sub.subarray().length, sub.subarray(-2).length, sub.subarray(2, 1).length, sub.subarray(4).length].join(','));
P('subarray write through', () => { sub.subarray(0, 2)[0] = 99; return sub[0]; });
P('subarray offset eq', () => { var x = Buffer.from([1,2,3,4]); return [x.subarray(2).offset === x.subarray(2).byteOffset, x.offset === x.byteOffset].join(','); });
P('subarray len/name/proto', () => [Buffer.prototype.subarray.length, Buffer.prototype.subarray.name, Buffer.prototype.subarray.hasOwnProperty('prototype')].join(','));
P('slice still Buffer', () => sub.slice(1, 2) instanceof Buffer);
P('toLocaleString', () => [Buffer.prototype.toLocaleString === Buffer.prototype.toString, Buffer.prototype.toLocaleString.name, Buffer.prototype.toLocaleString.length, Buffer.from('hi').toLocaleString()].join(','));
P('inspect name/len', () => [Buffer.prototype.inspect.name, Buffer.prototype.inspect.length].join(','));
P('inspect values', () => [Buffer.from([1,2,255]).inspect(), Buffer.alloc(0).inspect()].join('|'));
P('inspect call', () => Buffer.prototype.inspect.call(Buffer.from([0xab])));
P('inspect 50', () => Buffer.alloc(50, 1).inspect());
P('inspect 51', () => Buffer.alloc(51, 1).inspect());
P('inspect 60', () => Buffer.alloc(60, 1).inspect());
P('parent', () => { var x = Buffer.from([1]); return [x.parent === x.buffer, x.parent instanceof ArrayBuffer].join(','); });
P('parent desc', () => { var d = Object.getOwnPropertyDescriptor(Buffer.prototype, 'parent'); return [d.get.name, d.get.length, d.set, d.enumerable, d.configurable].join(','); });
P('offset desc', () => { var d = Object.getOwnPropertyDescriptor(Buffer.prototype, 'offset'); return [d.get.name, d.get.length, d.set, d.enumerable, d.configurable].join(','); });
P('offset eq', () => { var x = Buffer.from([1,2,3]); return [x.offset === x.byteOffset, x.subarray(1).offset === x.subarray(1).byteOffset].join(','); });
console.log(out.join('\n'));
"#;
const BUFFER_SUBARRAY_INSPECT_PARENT_OFFSET_MATCH_NODE_OUT: &str = r#"subarray 1..4 => ok:"true,true,3,2,Buffer,true"
subarray defaults => ok:"6,2,0,2"
subarray write through => ok:99
subarray offset eq => ok:"true,true"
subarray len/name/proto => ok:"2,subarray,true"
slice still Buffer => ok:true
toLocaleString => ok:"true,toString,3,hi"
inspect name/len => ok:"inspect,2"
inspect values => ok:"<Buffer 01 02 ff>|<Buffer >"
inspect call => ok:"<Buffer ab>"
inspect 50 => ok:"<Buffer 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01>"
inspect 51 => ok:"<Buffer 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 ... 1 more byte>"
inspect 60 => ok:"<Buffer 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 ... 10 more bytes>"
parent => ok:"true,true"
parent desc => ok:"get,0,,true,false"
offset desc => ok:"get,0,,true,false"
offset eq => ok:"true,true""#;
#[test]
fn buffer_subarray_inspect_parent_offset_match_node() {
    run_probe(
        BUFFER_SUBARRAY_INSPECT_PARENT_OFFSET_MATCH_NODE_JS,
        BUFFER_SUBARRAY_INSPECT_PARENT_OFFSET_MATCH_NODE_OUT,
    );
}
