use std::env;
use std::fs;
use std::io;
use std::io::Read;
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
use splayers::Entry;
use splayers::Status;

mod strings;

fn main() -> Result<(), Error> {
    let src = env::args().nth(1).ok_or(err_msg("first arg: src"))?;
    let dest = env::args_os().nth(2).ok_or(err_msg("second arg: dest"))?;
    let mut cwd = env::current_dir()?;
    cwd.push(dest);
    let dest = cwd;
    fs::create_dir_all(&dest)?;

    let src_url = url::Url::parse(&src)?;

    let path = src_url
        .path_segments()
        .ok_or(err_msg("not path"))?
        .last()
        .ok_or(err_msg("no end path"))?;

    let out = dest.join(&format!("{}.annul", path));

    if out.exists() {
        return Ok(());
    }

    let mut dsc = Vec::new();
    http_req::request::get(&src, &mut dsc).with_context(|_| err_msg("downloading dsc"))?;

    let sub_url = src_url.join(&path)?;

    let mut tmp = tempfile::NamedTempFile::new_in(dest)?;
    http_req::request::get(sub_url, &mut tmp).with_context(|_| err_msg("downloading"))?;

    let dictionary = if path.contains(".diff.") {
        &include_bytes!("../dicts/diff.zstd-dictionary")[..]
    } else if path.contains(".debian.") {
        &include_bytes!("../dicts/debian.tar.zstd-dictionary")[..]
    } else {
        &include_bytes!("../dicts/orig.zstd-dictionary")[..]
    };

    std::thread::Builder::new()
        .name(path.to_string())
        .spawn(move || unarchive(tmp.path(), &out, dictionary))?
        .join()
        .map_err(|_| err_msg("panic"))
        .with_context(|_| format_err!("processing {}", path))??;

    Ok(())
}

fn unarchive(src: &Path, dest: &Path, dictionary: &[u8]) -> Result<(), Error> {
    let root = dest.parent().ok_or(err_msg("root?"))?;

    let unpack =
        splayers::Unpack::unpack_into(src, &root).with_context(|_| err_msg("unpacking failed"))?;

    let out = tempfile_fast::PersistableTempFile::new_in(&root)?;

    let mut out = zstd::Encoder::with_dictionary(out, 8, dictionary)?;

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
            let mut stringed = tempfile::tempfile_in(temp.parent().unwrap())?;
            {
                let mut stringer = strings::StringBuf::new(io::BufWriter::new(&mut stringed));
                loop {
                    let mut buf = [0u8; 16 * 1024];
                    let len = file.read(&mut buf)?;
                    if 0 == len {
                        break;
                    }
                    let buf = &buf[..len];
                    stringer.accept(buf)?;
                }
                stringer.finish()?.flush()?;
            }
            let original_len = file.metadata()?.len();
            let new_len = stringed.metadata()?.len();
            if original_len == new_len {
                meta.push(0);
            } else {
                meta.push(1);
            }

            stringed.seek(SeekFrom::Start(0))?;

            Some((stringed, new_len))
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
