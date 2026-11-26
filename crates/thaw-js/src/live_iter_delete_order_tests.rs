//! `for-of` over a Set/Map/Array is live (entries added or removed mid-loop are seen), and `delete` on a large
//! object tombstones then compacts without changing key order. Expected strings are Node 22 output.
use super::*;

fn run(src: &str) -> String {
    let mut it = Interp::new();
    match it.run(src) {
        Ok(v) => it.to_str(&v).to_string(),
        Err(_) => panic!("program threw"),
    }
}

#[test]
fn for_of_sees_mutations_like_node() {
    assert_eq!(
        run(r####"const out=[];
const s=new Set();for(let i=1;i<=20;i++)s.add(i);
const seen=[];for(const v of s){seen.push(v);if(v<5)s.delete(v+1);if(v===6){for(let k=7;k<20;k++)s.delete(k);}}
out.push(seen.join(),s.size);
const s2=new Set([1,2]);let c=0;for(const v of s2){if(c++<5)s2.add(v+10);}out.push([...s2].join());
const m=new Map([[1,'a'],[2,'b'],[3,'c']]);const ms=[];for(const [k,v] of m){ms.push(k+v);if(k===1){m.delete(2);m.set(4,'d');}}out.push(ms.join());
const m2=new Map([[1,1],[2,2]]);const k2=[];for(const k of m2.keys()){k2.push(k);if(k===1){m2.delete(1);m2.set(1,9);}if(k2.length>4)break;}out.push(k2.join());
const a=[1,2,3];const as=[];for(const v of a){as.push(v);if(v===1)a.push(4);if(v===2)a.shift();}out.push(as.join(),a.join());
const a2=[1,2,3,4];const q=[];for(const v of a2){q.push(v);a2.length=2;}out.push(q.join());
function*g(){const t=new Set([1,2,3]);const r=[];for(const x of (yield* t,[])){} return 0}
function*g2(s){yield* s}
const s3=new Set([1,2,3,4]);const y=[];for(const v of g2(s3)){y.push(v);s3.delete(3);}out.push(y.join());
out.join('|');"####),
        r####"1,3,5,6,20|5|1,2,11,12,21,22,31|1a,3c,4d|1,2,1,1,1|1,2,4|2,3,4|1,2|1,2,4"####
    );
}

#[test]
fn delete_on_large_object_keeps_order_like_node() {
    assert_eq!(
        run(r####"const out=[];
const o={};for(let i=0;i<30;i++)o['k'+i]=i;for(let i=0;i<30;i+=3)delete o['k'+i];o.k0=1;o.k3=2;delete o.k4;o[5]=1;o[2]=2;
out.push(Object.keys(o).join());out.push(JSON.stringify(o));
const p={};for(let i=0;i<40;i++)p['a'+i]=i;for(let i=0;i<40;i++){if(i%2)delete p['a'+i];}
out.push(Object.keys(p).length,'a2' in p,'a3' in p,p.a4,p.a3,Object.entries(p).slice(0,3).join('|'));
for(let i=0;i<40;i++)delete p['a'+i];out.push(Object.keys(p).length);p.z=1;out.push(Object.keys(p).join());
const f=Object.freeze({a:1});out.push(delete f.a);const nc={};for(let i=0;i<20;i++)nc['x'+i]=i;Object.defineProperty(nc,'x7',{configurable:false});out.push(delete nc.x7,delete nc.x8,Object.keys(nc).length);
out.join('|');"####),
        r####"2,5,k1,k2,k5,k7,k8,k10,k11,k13,k14,k16,k17,k19,k20,k22,k23,k25,k26,k28,k29,k0,k3|{"2":2,"5":1,"k1":1,"k2":2,"k5":5,"k7":7,"k8":8,"k10":10,"k11":11,"k13":13,"k14":14,"k16":16,"k17":17,"k19":19,"k20":20,"k22":22,"k23":23,"k25":25,"k26":26,"k28":28,"k29":29,"k0":1,"k3":2}|20|true|false|4||a0,0|a2,2|a4,4|0|z|false|false|true|19"####
    );
}
