//! Synthetic production-workload page and clock endpoint, with no dependencies.
//! Run: cargo run --release --example latency_page -- 0.0.0.0:52415
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const PAGE: &str = r##"<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Mirror motion test</title><style>
body{margin:0;font:18px system-ui;background:#132137;color:white}header{position:fixed;z-index:2;top:0;left:0;right:0;background:#081423;padding:16px}
button{font:inherit;padding:8px}#content{padding:130px 16px 40px}article{padding:20px;margin:16px 0;border-radius:12px;background:#243d5b}
article:nth-child(3n){background:#36532b}article:nth-child(3n+1){background:#653e52}.bar{height:55px;background:linear-gradient(90deg,#2ac,#fc7,#c58);border-radius:8px}
#sync{font-size:13px;display:block}
</style><header><b>Mirror motion test</b><br><button id="toggle">Pause scrolling</button> <span id="counter"></span><span id="sync">Synchronizing clock…</span></header><main id="content"></main>
<canvas id="stamp" style="position:fixed;z-index:5;top:180px;left:0;width:100%;height:24px;image-rendering:pixelated" width="340" height="24"></canvas><script>
const content=document.querySelector('#content');
for(let i=0;i<100;i++)content.innerHTML+=`<article><h2>Scrolling section ${i+1}</h2><div class="bar"></div><p>A repeatable page for comparing phone mirroring. Clear text, colored panels and moving edges exercise the real screen encoder without personal content.</p></article>`;
let offset=null,best=Infinity;
async function sync(){
  const label=document.querySelector('#sync');
  offset=null;best=Infinity;
  for(let i=0;i<6;i++){
    try{
      const a=performance.now();
      const response=await fetch('/clock',{cache:'no-store',signal:AbortSignal.timeout(3000)});
      if(!response.ok)continue;
      const t=await response.json();const b=performance.now();
      if(Number.isFinite(t)&&b-a<best){best=b-a;offset=t-(a+b)/2;}
    }catch{}
  }
  label.textContent=offset===null?'Clock unavailable; motion test still running':`Clock sync RTT ${best.toFixed(2)} ms; midpoint uncertainty approximately ±${(best/2).toFixed(2)} ms`;
}
sync();
const cx=document.querySelector('#stamp').getContext('2d');
function stamp(t){if(offset===null){cx.clearRect(0,0,340,24);return;}const v=Math.round(t+offset)>>>0;for(let i=0;i<34;i++){cx.fillStyle=i===0?'#00ff00':i===33?'#ff00ff':((v>>>(32-i))&1)?'#ffffff':'#000000';cx.fillRect(i*10,0,10,24);}}
let wake=null;
async function keepAwake(){try{if('wakeLock' in navigator&&!wake){wake=await navigator.wakeLock.request('screen');wake.addEventListener('release',()=>{wake=null;});}}catch{}}
keepAwake();
document.addEventListener('visibilitychange',()=>{if(document.visibilityState==='visible'){keepAwake();sync();}});
let running=true,last=0,pos=0,direction=1;
document.querySelector('#toggle').onclick=()=>{running=!running;document.querySelector('#toggle').textContent=running?'Pause scrolling':'Resume scrolling';keepAwake();};
function frame(t){stamp(t);if(last&&running){pos+=direction*450*Math.min((t-last)/1000,.05);if(pos>document.body.scrollHeight-innerHeight-100)direction=-1;if(pos<0){pos=0;direction=1}scrollTo(0,pos)}last=t;document.querySelector('#counter').textContent=(t/1000).toFixed(1)+' s';requestAnimationFrame(frame)}requestAnimationFrame(frame);
</script>"##;

const HEADER_LIMIT: usize = 8 * 1024;

fn read_header(reader: &mut impl Read, storage: &mut [u8; HEADER_LIMIT]) -> io::Result<usize> {
    let mut used = 0;
    loop {
        if used == storage.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request header too large",
            ));
        }
        let count = reader.read(&mut storage[used..])?;
        if count == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        let old = used;
        used += count;
        // Resume three bytes before the new chunk to cover a split terminator.
        if let Some(end) = storage[old.saturating_sub(3)..used]
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
        {
            return Ok(old.saturating_sub(3) + end + 4);
        }
    }
}

fn response(stream: &mut impl Write, status: &str, kind: &str, body: &[u8]) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}

fn serve(mut stream: TcpStream) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.set_nodelay(true)?;
    let mut header = [0; HEADER_LIMIT];
    let length = match read_header(&mut stream, &mut header) {
        Ok(length) => length,
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            return response(
                &mut stream,
                "431 Request Header Fields Too Large",
                "text/plain",
                b"Header too large\n",
            );
        }
        Err(error) => return Err(error),
    };
    let first = header[..length]
        .split(|&b| b == b'\r')
        .next()
        .unwrap_or_default();
    let mut parts = first.split(|&b| b == b' ');
    let (method, path, version) = (parts.next(), parts.next(), parts.next());
    if !matches!(version, Some(b"HTTP/1.0" | b"HTTP/1.1")) || parts.next().is_some() {
        return response(
            &mut stream,
            "400 Bad Request",
            "text/plain",
            b"Bad request\n",
        );
    }
    if method != Some(b"GET") {
        return response(
            &mut stream,
            "405 Method Not Allowed",
            "text/plain",
            b"GET only\n",
        );
    }
    match path {
        Some(b"/") => response(
            &mut stream,
            "200 OK",
            "text/html; charset=utf-8",
            PAGE.as_bytes(),
        ),
        Some(b"/clock") => {
            let elapsed = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(io::Error::other)?;
            let milliseconds = elapsed.as_secs_f64() * 1000.0;
            response(
                &mut stream,
                "200 OK",
                "application/json",
                milliseconds.to_string().as_bytes(),
            )
        }
        _ => response(&mut stream, "404 Not Found", "text/plain", b"Not found\n"),
    }
}

fn main() -> io::Result<()> {
    let bind = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:52415".to_owned());
    let listener = Arc::new(TcpListener::bind(bind)?);
    println!(
        "Synthetic latency workload listening on {}",
        listener.local_addr()?
    );
    // Fixed worker count bounds concurrent connections, thread stacks and buffers.
    // Multiple workers prevent a speculative browser connection delaying /clock.
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
    fn fragmented_headers_are_bounded_and_terminated_exactly() -> io::Result<()> {
        struct Fragmented<'a>(&'a [u8]);
        impl Read for Fragmented<'_> {
            fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
                if self.0.is_empty() {
                    return Ok(0);
                }
                out[0] = self.0[0];
                self.0 = &self.0[1..];
                Ok(1)
            }
        }
        let mut storage = [0; HEADER_LIMIT];
        let bytes = b"GET /clock HTTP/1.1\r\nHost: local\r\n\r\ntrailing";
        let length = read_header(&mut Fragmented(bytes), &mut storage)?;
        assert_eq!(
            &storage[..length],
            b"GET /clock HTTP/1.1\r\nHost: local\r\n\r\n"
        );
        assert!(
            matches!(read_header(&mut &vec![b'x'; HEADER_LIMIT + 1][..], &mut storage), Err(e) if e.kind() == io::ErrorKind::InvalidData)
        );
        Ok(())
    }
}
