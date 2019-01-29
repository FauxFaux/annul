use std::env;
use std::fs;
use std::io;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::path::Path;

use byteorder::WriteBytesExt;
use byteorder::LE;
use cast::u64;
use failure::bail;
use failure::ensure;
use failure::err_msg;
use failure::format_err;
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
    fs::create_dir_all(&dest)?;

    let src_url = url::Url::parse(&src)?;

    let path = src_url.path_segments().ok_or(err_msg("not path"))?.last().ok_or(err_msg("no end path"))?;

    let out = dest.join(&format!("{}.annul", path));

    if out.exists() {
        return Ok(());
    }

    let mut dsc = Vec::new();
    http_req::request::get(&src, &mut dsc).with_context(|_| err_msg("downloading dsc"))?;

    let sub_url = src_url.join(&path)?;

    let mut tmp = tempfile::NamedTempFile::new()?;
    http_req::request::get(sub_url, &mut tmp).with_context(|_| err_msg("downloading"))?;

    unarchive(tmp.path(), &out).with_context(|_| format_err!("processing {}", path))?;

    Ok(())
}

fn unarchive(src: &Path, dest: &Path) -> Result<(), Error> {
    let root = dest.parent().ok_or(err_msg("root?"))?;

    let unpack =
        splayers::Unpack::unpack_into(src, &root).with_context(|_| err_msg("unpacking failed"))?;

    let out = tempfile_fast::PersistableTempFile::new_in(&root)?;

    let mut out = zstd::Encoder::new(out, 8)?;

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
        let mut meta = Vec::with_capacity(1 + name_prefix.len() + entry.local.path.len());

        let file = if let Some(temp) = entry.local.temp.as_ref() {
            let mut file = fs::File::open(temp)?;
            let mut header = [0u8; 64 * 1024];
            let read = file.read_many(&mut header)?;

            if likely_text(&header[..read]) {
                let data_len = file.metadata()?.len();

                file.seek(SeekFrom::Start(0))?;

                meta.push(0);
                Some((file, data_len))
            } else {
                meta.push(1);
                None
            }
        } else {
            meta.push(2);
            None
        };

        match &entry.children {
            Status::Unnecessary => meta.push(3),
            Status::Unrecognised => meta.push(4),
            Status::TooNested => meta.push(5),
            Status::Unsupported(_) => meta.push(6),
            Status::Error(_) => meta.push(7),
            Status::Success(_) => meta.push(8),
        }
        meta.extend_from_slice(&name_prefix);
        meta.extend_from_slice(&entry.local.path);

        // hmm, trying to make the name distinct from the content, for grepping
        meta.push(0);

        let data_len = file.as_ref().map(|(_file, size)| *size).unwrap_or(0);

        out.write_u64::<LE>(8 + data_len + u64(meta.len()))?;
        out.write_u64::<LE>(u64(meta.len()))?;
        out.write_all(&meta)?;

        if let Some((mut file, _)) = file {
            let written = io::copy(&mut file, out)?;
            ensure!(
                written == data_len,
                "short write: expected: {}, actual: {}",
                data_len,
                written
            );
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

    !buf.iter()
        .any(|&b| 4 == b || (b >= 5 && b <= 8) || (b >= 14 && b <= 26))
}
