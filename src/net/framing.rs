//! Uzunluk önekli çerçeveler: `[4 bayt büyük-endian uzunluk][gövde]`.
//! Referans: `reference/electron/src/core/framing.js`.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

/// 4 MB (dosya parçaları 256 KB olduğundan bol bol yeter).
pub const MAX_FRAME: usize = 4 * 1024 * 1024;

/// Gövdeyi uzunluk önekiyle birlikte tek bir tampona koyar.
pub fn encode_frame(payload: &[u8]) -> io::Result<Vec<u8>> {
    if payload.len() > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "çerçeve çok büyük"));
    }
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

/// Bir çerçeveyi tek `write_all` ile yazar (başlık ve gövde ayrı TCP paketlerine bölünmesin diye).
pub fn write_frame<W: Write>(w: &mut W, payload: &[u8]) -> io::Result<()> {
    let buf = encode_frame(payload)?;
    w.write_all(&buf)?;
    w.flush()
}

/// Bir çerçeve okur. Karşı taraf bağlantıyı tam çerçeve sınırında kapattıysa
/// `UnexpectedEof` döner. Aşırı büyük uzunluk önekinde bellek ayırmadan hata döner.
pub fn read_frame<R: Read>(r: &mut R) -> io::Result<Vec<u8>> {
    let mut head = [0u8; 4];
    r.read_exact(&mut head)?;
    let size = u32::from_be_bytes(head) as usize;
    if size > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "karşı taraf çok büyük çerçeve gönderdi"));
    }
    // Güvenilmez uzunluk: belleği okudukça büyüt (önceden 4 MB ayırma).
    let mut body = Vec::with_capacity(size.min(64 * 1024));
    let got = r.by_ref().take(size as u64).read_to_end(&mut body)?;
    if got != size {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "çerçeve yarıda kesildi"));
    }
    Ok(body)
}

/// Her `read` çağrısından önce kalan süreyi okuma zaman aşımı olarak ayarlayan sarmalayıcı.
/// Böylece yavaş damlatılan veriyle (slowloris) süre uzatılamaz.
pub struct DeadlineReader<'a> {
    pub stream: &'a TcpStream,
    pub deadline: Instant,
}

impl Read for DeadlineReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let now = Instant::now();
        if now >= self.deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "el sıkışma zaman aşımı"));
        }
        let left = (self.deadline - now).max(Duration::from_millis(1));
        self.stream.set_read_timeout(Some(left))?;
        match (&mut &*self.stream).read(buf) {
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                Err(io::Error::new(io::ErrorKind::TimedOut, "el sıkışma zaman aşımı"))
            }
            r => r,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Her `read` çağrısında en fazla 1 bayt veren okuyucu (parça parça gelen TCP akışı).
    struct Trickle(Vec<u8>, usize);
    impl Read for Trickle {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.1 >= self.0.len() || buf.is_empty() {
                return Ok(0);
            }
            buf[0] = self.0[self.1];
            self.1 += 1;
            Ok(1)
        }
    }

    #[test]
    fn frames_reassemble_byte_by_byte() {
        let mut all = encode_frame("merhaba".as_bytes()).unwrap();
        all.extend(encode_frame("dünya".as_bytes()).unwrap());
        all.extend(encode_frame(b"").unwrap());
        let mut r = Trickle(all, 0);
        assert_eq!(read_frame(&mut r).unwrap(), "merhaba".as_bytes());
        assert_eq!(read_frame(&mut r).unwrap(), "dünya".as_bytes());
        assert_eq!(read_frame(&mut r).unwrap(), b"");
        assert_eq!(read_frame(&mut r).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn oversize_rejected_both_ways() {
        assert!(encode_frame(&vec![0u8; MAX_FRAME + 1]).is_err());
        assert!(encode_frame(&vec![0u8; MAX_FRAME]).is_ok());
        let head = ((MAX_FRAME + 1) as u32).to_be_bytes().to_vec();
        let e = read_frame(&mut &head[..]).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidData);
        let head = u32::MAX.to_be_bytes().to_vec();
        assert!(read_frame(&mut &head[..]).is_err());
    }

    #[test]
    fn truncated_body_is_error() {
        let mut f = encode_frame(b"abcdef").unwrap();
        f.truncate(7);
        assert_eq!(read_frame(&mut &f[..]).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    }
}
