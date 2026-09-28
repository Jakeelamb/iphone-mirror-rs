//! Temporary synthetic multitouch diagnostic; no device connection or dependencies.
//! Run: cargo run --example touch_page -- --bind 0.0.0.0 --port 0
use std::io::{self, BufReader, Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

const HEADER_LIMIT: usize = 8192;
const BODY_LIMIT: usize = 2048;
const CONTACT_LIMIT: usize = 10;

const PAGE: &str = r##"<!doctype html><html><head>
<meta name="viewport" content="width=device-width,initial-scale=1,maximum-scale=1,user-scalable=no,viewport-fit=cover">
<title>Mirror touch test</title><style>
*{box-sizing:border-box;touch-action:none;-webkit-user-select:none;user-select:none}
html,body{margin:0;overflow:hidden;width:100%;height:100%;background:#111923;color:#edf2f7;font:16px system-ui}
canvas{position:fixed;inset:0;width:100%;height:100%}header{position:fixed;inset:0 0 auto;padding:18px 16px;background:#111923dd;pointer-events:none}
#counts{font-size:23px;font-weight:700}#events,#network{font-size:12px;margin-top:6px}#hint{font-size:12px;color:#afbecf;margin-top:6px}
</style></head><body><canvas id="surface"></canvas><header><div id="counts">Active 0 · maximum 0</div><div id="events"></div><div id="network">Log ready</div><div id="hint">Hold MOVE while touching LOOK or BUTTON. Release all: active must return to 0. Reload to reset.</div></header><script>
'use strict';
const canvas=document.querySelector('#surface'),ctx=canvas.getContext('2d');
const counts=document.querySelector('#counts'),events=document.querySelector('#events'),network=document.querySelector('#network');
const colors=['#4ed2b5','#ffb65d','#b296ff','#ff799b','#6cbfff','#edf277','#e69aff','#83e18d','#ffa582','#a3c5ed'];
let contacts=[],maximum=0,starts=0,ends=0,cancels=0,moves=0,sequence=0,lastMove=0,queue=[],sending=false,dropped=0,failed=0,sent=0;
function draw(){
 const w=innerWidth,h=innerHeight,dpr=devicePixelRatio||1;
 if(canvas.width!==Math.round(w*dpr)||canvas.height!==Math.round(h*dpr)){canvas.width=Math.round(w*dpr);canvas.height=Math.round(h*dpr);}
 ctx.setTransform(dpr,0,0,dpr,0,0);ctx.clearRect(0,0,w,h);
 ctx.fillStyle='#17352f';ctx.fillRect(0,h*.3,w*.48,h*.7);
 ctx.fillStyle='#232d4a';ctx.fillRect(w*.52,h*.3,w*.48,h*.7);
 ctx.fillStyle='#54312c';ctx.fillRect(w*.7,h*.7,w*.3,h*.3);
 ctx.fillStyle='#b7c9d6';ctx.font='bold 18px system-ui';ctx.textAlign='center';
 ctx.fillText('MOVE',w*.24,h*.58);ctx.fillText('LOOK',w*.76,h*.53);ctx.fillText('BUTTON',w*.84,h*.85);
 for(const t of contacts){ctx.fillStyle=colors[t.id%colors.length];ctx.beginPath();ctx.arc(t.x,t.y,25,0,2*Math.PI);ctx.fill();ctx.strokeStyle='white';ctx.lineWidth=2;ctx.stroke();ctx.fillStyle='#111923';ctx.font='bold 14px system-ui';ctx.fillText(String(t.id),t.x,t.y+5);}
 counts.textContent=`Active ${contacts.length} · maximum ${maximum}`;
 events.textContent=`Starts ${starts} · ends ${ends} · cancels ${cancels} · moves ${moves}`;
}
function logStatus(){network.textContent=`Logged ${sent} · pending ${queue.length} · failed ${failed} · dropped ${dropped}`;}
async function drain(){
 if(sending)return;sending=true;
 while(queue.length){const body=queue.shift();try{const r=await fetch('/touch',{method:'POST',headers:{'Content-Type':'text/plain'},body,signal:AbortSignal.timeout(2000)});if(!r.ok)throw new Error('log rejected');sent++;}catch{failed++;}logStatus();}
 sending=false;
}
function snapshot(kind){
 const header=[++sequence,kind,contacts.length,maximum,starts,ends,cancels,moves,innerWidth,innerHeight].join(' ');
 const body=header+'\n'+contacts.map(t=>[t.id,t.x.toFixed(1),t.y.toFixed(1)].join(' ')).join('\n');
 if(queue.length>=64){dropped++;logStatus();return;}queue.push(body);logStatus();drain();
}
for(const [event,kind] of [['touchstart','start'],['touchmove','move'],['touchend','end'],['touchcancel','cancel']]){
 addEventListener(event,e=>{e.preventDefault();contacts=Array.from(e.touches,t=>({id:t.identifier,x:Math.max(0,Math.min(innerWidth,t.clientX)),y:Math.max(0,Math.min(innerHeight,t.clientY))}));
 maximum=Math.max(maximum,contacts.length);if(kind==='start')starts+=e.changedTouches.length;else if(kind==='end')ends+=e.changedTouches.length;else if(kind==='cancel')cancels+=e.changedTouches.length;else moves++;
 draw();const now=performance.now();if(kind!=='move'||now-lastMove>=100){lastMove=now;snapshot(kind);}
 },{passive:false});
}
addEventListener('resize',draw);draw();snapshot('ready');
</script></body></html>"##;

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn header(reader: &mut impl Read, storage: &mut [u8; HEADER_LIMIT]) -> io::Result<usize> {
    // Buffered caller avoids per-byte socket reads and preserves prefetched body bytes.
    for used in 1..=storage.len() {
        reader.read_exact(&mut storage[used - 1..used])?;
        if used >= 4 && storage[used - 4..used] == *b"\r\n\r\n" {
            return Ok(used);
        }
    }
    Err(invalid("header too large"))
}

#[derive(Debug)]
struct Snapshot<'a> {
    sequence: u32,
    kind: &'a str,
    active: usize,
    maximum: usize,
    counters: [u32; 4],
    size: [u32; 2],
    contacts: [(u32, f32, f32); CONTACT_LIMIT],
}

fn snapshot(body: &str) -> io::Result<Snapshot<'_>> {
    let mut words = body.split_ascii_whitespace();
    let number = |words: &mut std::str::SplitAsciiWhitespace<'_>| -> io::Result<u32> {
        words
            .next()
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| invalid("expected integer"))
    };
    let sequence = number(&mut words)?;
    let kind = words.next().ok_or_else(|| invalid("missing event"))?;
    if !matches!(kind, "ready" | "start" | "move" | "end" | "cancel") {
        return Err(invalid("unknown event"));
    }
    let active = number(&mut words)? as usize;
    let maximum = number(&mut words)? as usize;
    if active > CONTACT_LIMIT || maximum > CONTACT_LIMIT || active > maximum {
        return Err(invalid("invalid contact count"));
    }
    let counters = [
        number(&mut words)?,
        number(&mut words)?,
        number(&mut words)?,
        number(&mut words)?,
    ];
    let size = [number(&mut words)?, number(&mut words)?];
    if size.iter().any(|v| !(1..=10000).contains(v)) {
        return Err(invalid("invalid viewport"));
    }
    let mut contacts = [(0, 0.0, 0.0); CONTACT_LIMIT];
    for index in 0..active {
        let id = number(&mut words)?;
        if contacts[..index]
            .iter()
            .any(|(previous, _, _)| *previous == id)
        {
            return Err(invalid("duplicate contact"));
        }
        let mut coordinates = [0.0_f32; 2];
        for axis in 0..2 {
            coordinates[axis] = words
                .next()
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| invalid("invalid coordinate"))?;
            if !coordinates[axis].is_finite()
                || !(0.0..=size[axis] as f32).contains(&coordinates[axis])
            {
                return Err(invalid("coordinate outside viewport"));
            }
        }
        contacts[index] = (id, coordinates[0], coordinates[1]);
    }
    if words.next().is_some() {
        return Err(invalid("extra snapshot fields"));
    }
    Ok(Snapshot {
        sequence,
        kind,
        active,
        maximum,
        counters,
        size,
        contacts,
    })
}

fn response(stream: &mut impl Write, status: &str, kind: &str, body: &[u8]) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}

fn request(reader: &mut impl Read, writer: &mut impl Write) -> io::Result<()> {
    let mut storage = [0; HEADER_LIMIT];
    let length = header(reader, &mut storage)?;
    let text = std::str::from_utf8(&storage[..length]).map_err(|_| invalid("invalid header"))?;
    let mut lines = text.split("\r\n");
    let mut first = lines.next().unwrap_or_default().split_ascii_whitespace();
    let (method, path, version) = (first.next(), first.next(), first.next());
    if !matches!(version, Some("HTTP/1.0" | "HTTP/1.1")) || first.next().is_some() {
        return Err(invalid("invalid request line"));
    }
    let mut length = None;
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| invalid("invalid header field"))?;
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(invalid("chunked bodies unsupported"));
        }
        if name.eq_ignore_ascii_case("content-length") {
            if length.is_some() {
                return Err(invalid("duplicate content length"));
            }
            length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| invalid("invalid content length"))?,
            );
        }
    }
    if length.is_some_and(|length| length > BODY_LIMIT) {
        return Err(invalid("body too large"));
    }
    match (method, path) {
        (Some("GET"), Some("/")) => response(
            writer,
            "200 OK",
            "text/html; charset=utf-8",
            PAGE.as_bytes(),
        ),
        (Some("POST"), Some("/touch")) => {
            let length = length.ok_or_else(|| invalid("missing content length"))?;
            let mut body = [0; BODY_LIMIT];
            reader.read_exact(&mut body[..length])?;
            let state = snapshot(
                std::str::from_utf8(&body[..length]).map_err(|_| invalid("invalid snapshot"))?,
            )?;
            println!(
                "touch seq={} event={} active={} max={} starts={} ends={} cancels={} moves={} viewport={}x{} contacts={:?}",
                state.sequence,
                state.kind,
                state.active,
                state.maximum,
                state.counters[0],
                state.counters[1],
                state.counters[2],
                state.counters[3],
                state.size[0],
                state.size[1],
                &state.contacts[..state.active]
            );
            response(writer, "204 No Content", "text/plain", b"")
        }
        _ => response(writer, "404 Not Found", "text/plain", b"Not found\n"),
    }
}

fn serve(mut stream: TcpStream) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    match request(&mut reader, &mut stream) {
        Err(error) if error.kind() == io::ErrorKind::InvalidData => response(
            &mut stream,
            "400 Bad Request",
            "text/plain",
            b"Invalid or oversized diagnostic request\n",
        ),
        result => result,
    }
}

fn main() -> io::Result<()> {
    let mut bind: IpAddr = "127.0.0.1".parse().map_err(io::Error::other)?;
    let mut port = 0_u16;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--bind" => {
                bind = arguments
                    .next()
                    .ok_or_else(|| invalid("--bind requires an IP address"))?
                    .parse()
                    .map_err(io::Error::other)?
            }
            "--port" => {
                port = arguments
                    .next()
                    .ok_or_else(|| invalid("--port requires a port"))?
                    .parse()
                    .map_err(io::Error::other)?
            }
            _ => return Err(invalid("usage: touch_page [--bind IP] [--port PORT]")),
        }
    }
    let listener = Arc::new(TcpListener::bind((bind, port))?);
    println!(
        "Synthetic touch test listening on http://{}",
        listener.local_addr()?
    );
    let mut workers = Vec::with_capacity(4);
    for _ in 0..4 {
        let listener = Arc::clone(&listener);
        workers.push(std::thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let _ = serve(stream);
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        }));
    }
    for worker in workers {
        worker
            .join()
            .map_err(|_| io::Error::other("HTTP worker stopped"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_contacts_and_final_release_are_preserved() -> io::Result<()> {
        let state = snapshot("3 start 2 2 2 0 0 4 400 800\n7 100.5 600\n9 300 200")?;
        assert_eq!(state.active, 2);
        assert_eq!(
            &state.contacts[..2],
            &[(7, 100.5, 600.0), (9, 300.0, 200.0)]
        );
        let state = snapshot("4 end 0 2 2 2 0 4 400 800")?;
        assert_eq!(state.active, 0);
        assert_eq!(state.maximum, 2);
        Ok(())
    }

    #[test]
    fn malformed_or_unbounded_snapshots_are_rejected() {
        for body in [
            "",
            "1 fake 0 0 0 0 0 0 400 800",
            "1 start 11 11 11 0 0 0 400 800",
            "1 start 1 1 1 0 0 0 400 800 1 NaN 3",
            "1 start 1 1 1 0 0 0 400 800 1 401 3",
            "1 start 2 2 2 0 0 0 400 800 1 1 2 1 3 4",
            "1 end 0 1 1 1 0 0 400 800 extra",
        ] {
            assert!(snapshot(body).is_err(), "accepted {body}");
        }
    }

    #[test]
    fn http_consumes_body_and_bounds_headers_and_lengths() -> io::Result<()> {
        let body = "1 end 0 2 2 2 0 0 400 800";
        let input = format!(
            "POST /touch HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let mut output = Vec::new();
        request(&mut input.as_bytes(), &mut output)?;
        assert!(output.starts_with(b"HTTP/1.1 204"));
        for input in [
            "POST /touch HTTP/1.1\r\nContent-Length: 2049\r\n\r\n",
            "POST /touch HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n",
            "POST /touch HTTP/1.1\r\nContent-Length: 0\r\nContent-Length: 0\r\n\r\n",
        ] {
            assert!(request(&mut input.as_bytes(), &mut Vec::new()).is_err());
        }
        assert!(header(&mut &[b'x'; HEADER_LIMIT][..], &mut [0; HEADER_LIMIT]).is_err());
        Ok(())
    }
}
