//! Asking an SMB server for its shares: one DCE/RPC call, `NetrShareEnum`
//! level 1, over the `srvsvc` pipe.
//!
//! By hand rather than through smb-rs's own `list_shares`, which binds with
//! the NDR64 transfer syntax only. Samba refuses NDR64, and Samba is what
//! nearly every home NAS runs; the classic NDR syntax used here is the one
//! every server, Windows included, accepts. Only what this one call needs is
//! encoded: a bind, one request, and the reply's array of names.
//!
//! References: [C706] for the PDUs and NDR, [MS-SRVS] 3.1.4.8 for the call.

/// `srvsvc`, interface version 3.0.
const SRVSVC: [u8; 16] = guid(
    0x4b32_4fc8,
    0x1670,
    0x01d3,
    [0x12, 0x78, 0x5a, 0x47, 0xbf, 0x6e, 0xe1, 0x88],
);
/// Classic NDR, transfer syntax version 2.
const NDR: [u8; 16] = guid(
    0x8a88_5d04,
    0x1ceb,
    0x11c9,
    [0x9f, 0xe8, 0x08, 0x00, 0x2b, 0x10, 0x48, 0x60],
);
/// `NetrShareEnum`.
const OPNUM: u16 = 15;
/// The largest fragment either side is asked to send.
pub const MOST_FRAGMENT: u16 = 4280;

const BIND: u8 = 11;
const BIND_ACK: u8 = 12;
const REQUEST: u8 = 0;
const RESPONSE: u8 = 2;
const FAULT: u8 = 3;
const FIRST_AND_LAST: u8 = 0x03;
const LAST_FRAG: u8 = 0x02;

/// A share's type: disk shares are 0 in the low bits; `$` admin shares carry
/// the special bit as well as their name.
const TYPE_MASK: u32 = 0x0FFF_FFFF;
const SPECIAL: u32 = 0x8000_0000;

/// A GUID as it is written on the wire: the first three fields little-endian.
const fn guid(a: u32, b: u16, c: u16, d: [u8; 8]) -> [u8; 16] {
    let a = a.to_le_bytes();
    let b = b.to_le_bytes();
    let c = c.to_le_bytes();
    [
        a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d[0], d[1], d[2], d[3], d[4], d[5], d[6],
        d[7],
    ]
}

/// The 16 bytes every connection-oriented PDU starts with. Little-endian,
/// ASCII, IEEE floats: `drep` 0x10.
fn header(kind: u8, call: u32, body_len: usize) -> Vec<u8> {
    let frag = u16::try_from(16 + body_len).unwrap_or(u16::MAX);
    let mut out = Vec::with_capacity(16 + body_len);
    out.extend_from_slice(&[5, 0, kind, FIRST_AND_LAST, 0x10, 0, 0, 0]);
    out.extend_from_slice(&frag.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&call.to_le_bytes());
    out
}

/// Binds the pipe to `srvsvc` in classic NDR.
#[must_use]
pub fn bind(call: u32) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&MOST_FRAGMENT.to_le_bytes());
    body.extend_from_slice(&MOST_FRAGMENT.to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes()); // a new association
    body.extend_from_slice(&[1, 0, 0, 0]); // one context
    body.extend_from_slice(&[0, 0, 1, 0]); // context 0, one transfer syntax
    body.extend_from_slice(&SRVSVC);
    body.extend_from_slice(&[3, 0, 0, 0]); // version 3.0
    body.extend_from_slice(&NDR);
    body.extend_from_slice(&2u32.to_le_bytes());
    let mut pdu = header(BIND, call, body.len());
    pdu.extend(body);
    pdu
}

/// Whether the server accepted the bind.
///
/// # Errors
/// A sentence on what the server said instead.
pub fn bound(reply: &[u8]) -> Result<(), String> {
    let kind = *reply.get(2).ok_or("an empty answer to the bind")?;
    if kind != BIND_ACK {
        return Err(format!(
            "the server answered the bind with packet type {kind}"
        ));
    }
    let mut at = Reader::new(reply, 16);
    at.skip(8)?; // fragment sizes and association
    let port = usize::from(at.u16()?);
    at.skip(port)?;
    at.align_from_start(4);
    let results = at.u8()?;
    at.skip(3)?;
    if results == 0 || at.u16()? != 0 {
        return Err("the server would not speak NDR on its share list".into());
    }
    Ok(())
}

/// `NetrShareEnum(NULL, level 1, no buffer, as much as it likes, no resume)`.
#[must_use]
pub fn ask(call: u32) -> Vec<u8> {
    let mut stub = Vec::new();
    for word in [
        0,           // ServerName: null, this server
        1,           // InfoStruct.Level
        1,           // the union's switch, the same level
        0x0002_0000, // a pointer to the level-1 container
        0,           // container: no entries read yet
        0,           // container: no buffer yet
        u32::MAX,    // PreferedMaximumLength: everything
        0,           // ResumeHandle: null
    ] {
        stub.extend_from_slice(&u32::to_le_bytes(word));
    }
    let mut body = Vec::new();
    body.extend_from_slice(&u32::try_from(stub.len()).unwrap_or(0).to_le_bytes()); // alloc hint
    body.extend_from_slice(&0u16.to_le_bytes()); // context 0
    body.extend_from_slice(&OPNUM.to_le_bytes());
    body.extend(stub);
    let mut pdu = header(REQUEST, call, body.len());
    pdu.extend(body);
    pdu
}

/// The disk shares in the reply, hidden ones (`$`) left out, unsorted.
///
/// # Errors
/// A sentence on what went wrong: a fault, a reply split over fragments (a
/// server with more shares than fit in one, which the form then asks to be
/// typed), or a reply that does not parse.
pub fn shares(reply: &[u8]) -> Result<Vec<String>, String> {
    let kind = *reply.get(2).ok_or("an empty answer to the share list")?;
    if kind == FAULT {
        return Err("the server refused to list its shares".into());
    }
    if kind != RESPONSE {
        return Err(format!(
            "the server answered the share list with packet type {kind}"
        ));
    }
    if reply.get(3).is_none_or(|flags| flags & LAST_FRAG == 0) {
        return Err("the server has more shares than fit in one answer".into());
    }
    let mut at = Reader::new(reply, 24);
    let _level = at.u32()?;
    let _switch = at.u32()?;
    if at.u32()? == 0 {
        return Ok(Vec::new()); // no container
    }
    let count = at.u32()? as usize;
    if at.u32()? == 0 {
        return Ok(Vec::new()); // no array
    }
    let max = at.u32()? as usize;
    if max < count || count > 4096 {
        return Err("the share list does not add up".into());
    }
    let mut rows = Vec::with_capacity(count);
    for _ in 0..count {
        let name = at.u32()?;
        let kind = at.u32()?;
        let remark = at.u32()?;
        rows.push((name != 0, kind, remark != 0));
    }
    // The strings follow the array, in order, for each pointer that was set.
    let mut out = Vec::new();
    for (has_name, kind, has_remark) in rows {
        let name = if has_name { Some(at.string()?) } else { None };
        if has_remark {
            at.string()?;
        }
        if let Some(name) = name
            && kind & TYPE_MASK == 0
            && kind & SPECIAL == 0
            && !name.ends_with('$')
        {
            out.push(name);
        }
    }
    Ok(out)
}

/// Reads little-endian NDR from a PDU, with offsets from the PDU's start
/// (the stub starts 8-aligned, so alignment from either is the same).
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8], at: usize) -> Self {
        Self { bytes, at }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self
            .at
            .checked_add(n)
            .filter(|end| *end <= self.bytes.len());
        let Some(end) = end else {
            return Err("the server's answer ends too soon".into());
        };
        let got = &self.bytes[self.at..end];
        self.at = end;
        Ok(got)
    }

    fn skip(&mut self, n: usize) -> Result<(), String> {
        self.take(n).map(|_| ())
    }

    fn align_from_start(&mut self, to: usize) {
        self.at = self.at.next_multiple_of(to);
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, String> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, String> {
        self.align_from_start(4);
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// A conformant varying UTF-16 string: maximum, offset, length, then the
    /// characters, the terminator among them.
    fn string(&mut self) -> Result<String, String> {
        let _max = self.u32()?;
        let _offset = self.u32()?;
        let len = self.u32()? as usize;
        let raw = self.take(len.checked_mul(2).ok_or("a share name too long")?)?;
        let units: Vec<u16> = raw
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        Ok(String::from_utf16_lossy(&units)
            .trim_end_matches('\0')
            .to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reply as Samba would write it, built from the same rules.
    fn reply(entries: &[(&str, u32, &str)]) -> Vec<u8> {
        let mut stub = Vec::new();
        let word = |stub: &mut Vec<u8>, w: u32| stub.extend_from_slice(&w.to_le_bytes());
        let string = |stub: &mut Vec<u8>, s: &str| {
            let units: Vec<u16> = s.encode_utf16().chain([0]).collect();
            let n = u32::try_from(units.len()).unwrap();
            for w in [n, 0, n] {
                stub.extend_from_slice(&w.to_le_bytes());
            }
            for u in units {
                stub.extend_from_slice(&u.to_le_bytes());
            }
            while !stub.len().is_multiple_of(4) {
                stub.push(0);
            }
        };
        let n = u32::try_from(entries.len()).unwrap();
        for w in [1, 1, 0x0002_0000, n, 0x0002_0004, n] {
            word(&mut stub, w);
        }
        for (i, (_, kind, _)) in entries.iter().enumerate() {
            let i = u32::try_from(i).unwrap();
            for w in [0x0002_0008 + i * 8, *kind, 0x0002_000c + i * 8] {
                word(&mut stub, w);
            }
        }
        for (name, _, remark) in entries {
            string(&mut stub, name);
            string(&mut stub, remark);
        }
        for w in [n, 0, 0] {
            word(&mut stub, w);
        }
        let mut body = Vec::new();
        body.extend_from_slice(&u32::try_from(stub.len()).unwrap().to_le_bytes());
        body.extend_from_slice(&[0, 0, 0, 0]);
        body.extend(stub);
        let mut pdu = header(RESPONSE, 3, body.len());
        pdu.extend(body);
        pdu
    }

    #[test]
    fn disk_shares_are_kept_and_hidden_ones_left_out() {
        let got = shares(&reply(&[
            ("Photos", 0, "Family photos"),
            ("IPC$", 3 | SPECIAL, "IPC Service"),
            ("C$", SPECIAL, ""),
            ("printers", 1, ""),
            ("Media", 0, ""),
        ]))
        .unwrap();
        assert_eq!(got, ["Photos", "Media"]);
    }

    #[test]
    fn an_odd_length_name_keeps_the_next_one_aligned() {
        let got = shares(&reply(&[("abc", 0, "x"), ("Backup", 0, "")])).unwrap();
        assert_eq!(got, ["abc", "Backup"]);
    }

    #[test]
    fn a_cut_short_reply_is_an_error_not_a_panic() {
        let full = reply(&[("Photos", 0, "")]);
        for cut in [3, 24, 40, full.len() - 20] {
            assert!(shares(&full[..cut]).is_err(), "cut at {cut}");
        }
    }

    #[test]
    fn a_fault_or_a_second_fragment_is_said_in_words() {
        let mut fault = reply(&[]);
        fault[2] = FAULT;
        assert!(shares(&fault).unwrap_err().contains("refused"));
        let mut split = reply(&[("Photos", 0, "")]);
        split[3] = 0x01;
        assert!(shares(&split).unwrap_err().contains("more shares"));
    }

    #[test]
    fn the_bind_names_srvsvc_in_classic_ndr() {
        let pdu = bind(2);
        assert_eq!(pdu[2], BIND);
        assert_eq!(usize::from(u16::from_le_bytes([pdu[8], pdu[9]])), pdu.len());
        assert!(pdu.windows(16).any(|w| w == SRVSVC));
        assert!(pdu.windows(16).any(|w| w == NDR));
    }

    #[test]
    fn a_rejected_bind_is_refused() {
        let mut ack = header(BIND_ACK, 2, 0);
        ack.extend_from_slice(&[0; 8]);
        ack.extend_from_slice(&4u16.to_le_bytes());
        ack.extend_from_slice(b"\\PIPE\\srvsvc\0"[..4].as_ref());
        while !ack.len().is_multiple_of(4) {
            ack.push(0);
        }
        ack.extend_from_slice(&[1, 0, 0, 0]);
        ack.extend_from_slice(&2u16.to_le_bytes()); // provider rejection
        ack.extend_from_slice(&[0; 22]);
        assert!(bound(&ack).is_err());
        let at = ack.len() - 24;
        ack[at] = 0;
        assert!(bound(&ack).is_ok());
    }
}
