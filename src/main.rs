use std::env;
use std::fs;
use std::io;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;

use byteorder::WriteBytesExt;
use byteorder::LE;
use cast::u64;
use failure::bail;
use failure::ensure;
use failure::err_msg;
use failure::Error;
use failure::ResultExt;
use iowrap::ReadMany;
use splayers::Entry;
use splayers::Status;

fn main() -> Result<(), Error> {
    let src = env::args().nth(1).ok_or(err_msg("first arg: src"))?;
    let dest = env::args_os().nth(2).ok_or(err_msg("second arg: dest"))?;
    let mut cwd = env::current_dir()?;
    cwd.push(dest);
    let dest = cwd;
    let root = dest.parent().ok_or(err_msg("file output please"))?;

    let mut tmp = tempfile::NamedTempFile::new()?;
    http_req::request::get(src, &mut tmp).with_context(|_| err_msg("downloading"))?;

    let unpack = splayers::Unpack::unpack_into(tmp.path(), root)
        .with_context(|_| err_msg("unpacking failed"))?;

    let out = tempfile_fast::PersistableTempFile::new_in(root)?;

    let mut out = zstd::Encoder::new(out, 0)?;

    match *unpack.status() {
        splayers::Status::Success(ref entries) => output(entries, &[], &mut out)?,
        ref other => bail!("expecting top level archive, not: {:?}", other),
    }

    let out = out.finish()?;

    out.persist_noclobber(dest).map_err(|e| e.error)?;

    Ok(())
}

fn output<W: Write>(entries: &[Entry], paths: &[Box<[u8]>], out: &mut W) -> Result<(), Error> {
    let mut entries: Vec<&Entry> = entries.iter().collect();

    let mut name_prefix = Vec::with_capacity(paths.len() * 128);
    for path in paths {
        name_prefix.extend_from_slice(path);
        name_prefix.push(0);
    }

    entries.sort_by_key(|e| e.local.path.as_ref());

    for entry in entries {
        let mut full_name = name_prefix.to_vec();
        full_name.extend_from_slice(&entry.local.path);

        // hmm, trying to make the name distinct from the content, for grepping
        full_name.push(0);

        out.write_u64::<LE>(u64(full_name.len()))?;

        if let Some(temp) = entry.local.temp.as_ref() {
            let mut file = fs::File::open(temp)?;
            let mut header = [0u8; 8 * 1024];
            let read = file.read_many(&mut header)?;

            if likely_text(&header[..read]) {
                let data_len = file.metadata()?.len();
                out.write_u64::<LE>(data_len)?;
                out.write_all(&full_name)?;

                file.seek(SeekFrom::Start(0))?;

                let written = io::copy(&mut file, out)?;
                ensure!(
                    written == data_len,
                    "short write: expected: {}, actual: {}",
                    data_len,
                    written
                );
            } else {
                out.write_u64::<LE>(0)?;
                out.write_all(&full_name)?;
            }
        } else {
            out.write_u64::<LE>(0)?;
            out.write_all(&full_name)?;
        }

        match &entry.children {
            Status::Success(entries) => {
                let mut paths = paths.to_vec();
                paths.push(entry.local.path.clone());
                output(&entries, &paths, out)?;
            }
            _ => (),
        }
    }
    Ok(())
}

fn likely_text(buf: &[u8]) -> bool {
    if memchr::memchr(0, buf).is_some() {
        return false;
    }

    !buf.iter().any(|&b| 4 == b || (b >= 5 && b <= 8) || (b >= 14 && b <= 26))
}
