//! The helper's pipe protocol, little-endian.
//!
//! - The helper first writes one byte, its protocol [`VERSION`], once its fonts are
//!   loaded. The first helpers wrote `R` (version 1).
//! - A request: the job's id (`u64`), kind (`u8`), two numbers (`u32`, `u32`), a fit
//!   (`u8`), then the bytes (`u32` length, at most [`MAX_JOB_BYTES`]).
//! - A reply, one per request: status (`u8`: 0 done, 1 refused), the id of the job it
//!   answers (`u64`), the job's CPU time (`u64` µs), the helper's CPU time so far (`u64`
//!   ms); then for a done job its width and height (`u32`, at most [`MAX_REPLY_SIDE`]) and
//!   their RGBA pixels, for a refused one the reason (`u32` length, at most
//!   [`MAX_REASON_BYTES`], UTF-8).
//!
//! Anything else is malformed: the helper stops reading, and the client stops the helper.
//! So is a reply while no job waits for one, a second reply, and a reply to another job.

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::time::Duration;

use tomakast_core::ImageFit;

use crate::{Job, Pixels};

/// The protocol's version: the byte the helper writes once it can take jobs.
pub const VERSION: u8 = 2;
/// The most bytes a job carries: more than any widget image (2 MiB).
pub const MAX_JOB_BYTES: u32 = 4 << 20;
/// The largest width or height of a reply.
pub const MAX_REPLY_SIDE: u32 = 4096;
/// The longest reason a refusal gives.
pub const MAX_REASON_BYTES: u32 = 4096;

const BITMAP: u8 = 1;
const SVG: u8 = 2;
const SLEEP: u8 = 100;
const ALLOCATE: u8 = 101;
const PANIC: u8 = 102;

/// What the helper answers to one job.
#[derive(Debug)]
pub struct Reply {
    /// The id of the job it answers.
    pub job: u64,
    pub outcome: Result<Pixels, String>,
    /// The helper's CPU time so far.
    pub helper_cpu: Duration,
}

fn malformed(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_string())
}

fn read_u8(input: &mut impl Read) -> io::Result<u8> {
    let mut b = [0; 1];
    input.read_exact(&mut b)?;
    Ok(b[0])
}

fn read_u32(input: &mut impl Read) -> io::Result<u32> {
    let mut b = [0; 4];
    input.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn read_u64(input: &mut impl Read) -> io::Result<u64> {
    let mut b = [0; 8];
    input.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}

fn read_bytes(input: &mut impl Read, len: u32, max: u32) -> io::Result<Vec<u8>> {
    if len > max {
        return Err(malformed("too long"));
    }
    let mut bytes = vec![0; len as usize];
    input.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn fit_tag(fit: ImageFit) -> u8 {
    match fit {
        ImageFit::Contain => 0,
        ImageFit::Cover => 1,
        ImageFit::Fill => 2,
    }
}

/// Writes `job`, as job `id`.
pub fn write_job(output: &mut impl Write, id: u64, job: &Job) -> io::Result<()> {
    let empty: Arc<[u8]> = Arc::from(&[][..]);
    let (kind, a, b, c, bytes) = match job {
        Job::Bitmap(bytes) => (BITMAP, 0, 0, 0, bytes.clone()),
        Job::Svg { bytes, device, fit } => (SVG, device.0, device.1, fit_tag(*fit), bytes.clone()),
        Job::Sleep(long) => (SLEEP, long.as_millis() as u32, 0, 0, empty),
        Job::Allocate(mib) => (ALLOCATE, *mib, 0, 0, empty),
        Job::Panic => (PANIC, 0, 0, 0, empty),
    };
    let len = u32::try_from(bytes.len())
        .ok()
        .filter(|len| *len <= MAX_JOB_BYTES)
        .ok_or_else(|| malformed("a job too long"))?;
    output.write_all(&id.to_le_bytes())?;
    output.write_all(&[kind])?;
    output.write_all(&a.to_le_bytes())?;
    output.write_all(&b.to_le_bytes())?;
    output.write_all(&[c])?;
    output.write_all(&len.to_le_bytes())?;
    output.write_all(&bytes)?;
    output.flush()
}

/// Reads a job and its id; `None` once the input ends between jobs.
pub fn read_job(input: &mut impl Read) -> io::Result<Option<(u64, Job)>> {
    let mut id = [0; 8];
    match input.read_exact(&mut id) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let id = u64::from_le_bytes(id);
    let kind = read_u8(input)?;
    let a = read_u32(input)?;
    let b = read_u32(input)?;
    let c = read_u8(input)?;
    let len = read_u32(input)?;
    let bytes: Arc<[u8]> = Arc::from(read_bytes(input, len, MAX_JOB_BYTES)?);
    Ok(Some((
        id,
        match kind {
            BITMAP => Job::Bitmap(bytes),
            SVG => Job::Svg {
                bytes,
                device: (a, b),
                fit: match c {
                    0 => ImageFit::Contain,
                    1 => ImageFit::Cover,
                    2 => ImageFit::Fill,
                    _ => return Err(malformed("an unknown fit")),
                },
            },
            SLEEP => Job::Sleep(Duration::from_millis(u64::from(a))),
            ALLOCATE => Job::Allocate(a),
            PANIC => Job::Panic,
            _ => return Err(malformed("an unknown job")),
        },
    )))
}

/// Writes that the helper is ready, and its version.
pub fn write_ready(output: &mut impl Write) -> io::Result<()> {
    output.write_all(&[VERSION])?;
    output.flush()
}

/// Reads that the helper is ready: the version it speaks.
pub fn read_ready(input: &mut impl Read) -> io::Result<u8> {
    read_u8(input)
}

/// Writes the answer to job `id`.
pub fn write_reply(
    output: &mut impl Write,
    id: u64,
    outcome: &Result<Pixels, String>,
    helper_cpu: Duration,
) -> io::Result<()> {
    let job_cpu = match outcome {
        Ok(pixels) => pixels.cpu,
        Err(_) => Duration::ZERO,
    };
    let status: u8 = if outcome.is_ok() { 0 } else { 1 };
    output.write_all(&[status])?;
    output.write_all(&id.to_le_bytes())?;
    output.write_all(&(job_cpu.as_micros() as u64).to_le_bytes())?;
    output.write_all(&(helper_cpu.as_millis() as u64).to_le_bytes())?;
    match outcome {
        Ok(pixels) => {
            output.write_all(&pixels.width.to_le_bytes())?;
            output.write_all(&pixels.height.to_le_bytes())?;
            output.write_all(&pixels.rgba)?;
        }
        Err(reason) => {
            let mut end = reason.len().min(MAX_REASON_BYTES as usize);
            while !reason.is_char_boundary(end) {
                end -= 1;
            }
            output.write_all(&(end as u32).to_le_bytes())?;
            output.write_all(&reason.as_bytes()[..end])?;
        }
    }
    output.flush()
}

/// Reads the answer to one job.
pub fn read_reply(input: &mut impl Read) -> io::Result<Reply> {
    let status = read_u8(input)?;
    read_reply_after(status, input)
}

/// Reads the rest of a reply whose status was read.
pub fn read_reply_after(status: u8, input: &mut impl Read) -> io::Result<Reply> {
    let job = read_u64(input)?;
    let job_cpu = Duration::from_micros(read_u64(input)?);
    let helper_cpu = Duration::from_millis(read_u64(input)?);
    let outcome = match status {
        0 => {
            let width = read_u32(input)?;
            let height = read_u32(input)?;
            if width > MAX_REPLY_SIDE || height > MAX_REPLY_SIDE {
                return Err(malformed("a reply too large"));
            }
            let len = width * height * 4;
            let rgba = read_bytes(input, len, len)?;
            Ok(Pixels {
                width,
                height,
                rgba,
                cpu: job_cpu,
            })
        }
        1 => {
            let len = read_u32(input)?;
            let reason = read_bytes(input, len, MAX_REASON_BYTES)?;
            Err(String::from_utf8(reason).map_err(|_| malformed("a reason not UTF-8"))?)
        }
        _ => return Err(malformed("an unknown status")),
    };
    Ok(Reply {
        job,
        outcome,
        helper_cpu,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_and_replies_round_trip() {
        let jobs = [
            Job::Bitmap(Arc::from(&b"png"[..])),
            Job::Svg {
                bytes: Arc::from(&b"<svg/>"[..]),
                device: (710, 240),
                fit: ImageFit::Cover,
            },
            Job::Sleep(Duration::from_millis(5)),
            Job::Allocate(3),
            Job::Panic,
        ];
        let mut wire = Vec::new();
        for (id, job) in jobs.iter().enumerate() {
            write_job(&mut wire, id as u64 + 40, job).unwrap();
        }
        let mut input = &wire[..];
        for (id, job) in jobs.iter().enumerate() {
            let read = read_job(&mut input).unwrap().unwrap();
            assert_eq!((read.0, &read.1), (id as u64 + 40, job));
        }
        assert!(read_job(&mut input).unwrap().is_none());

        let pixels = Pixels {
            width: 2,
            height: 1,
            rgba: vec![1, 2, 3, 4, 5, 6, 7, 8],
            cpu: Duration::from_micros(42),
        };
        let mut wire = Vec::new();
        write_reply(&mut wire, 7, &Ok(pixels.clone()), Duration::from_millis(9)).unwrap();
        write_reply(&mut wire, 8, &Err("no".into()), Duration::from_millis(10)).unwrap();
        let mut input = &wire[..];
        let done = read_reply(&mut input).unwrap();
        assert_eq!(done.job, 7);
        assert_eq!(done.outcome, Ok(pixels));
        assert_eq!(done.helper_cpu, Duration::from_millis(9));
        assert_eq!(read_reply(&mut input).unwrap().outcome, Err("no".into()));
    }

    #[test]
    fn malformed_input_is_an_error() {
        let mut too_long = vec![0, 0, 0, 0, 0, 0, 0, 0, BITMAP, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        too_long.extend_from_slice(&(MAX_JOB_BYTES + 1).to_le_bytes());
        assert!(read_job(&mut &too_long[..]).is_err());
        let mut huge = vec![0u8; 25];
        huge.extend_from_slice(&(MAX_REPLY_SIDE + 1).to_le_bytes());
        huge.extend_from_slice(&1u32.to_le_bytes());
        assert!(read_reply(&mut &huge[..]).is_err());
    }
}
