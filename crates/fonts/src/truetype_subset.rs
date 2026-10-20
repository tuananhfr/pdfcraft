//! Sparse TrueType subsetting for the bundled static Inter face; glyph IDs remain stable.

use std::collections::{BTreeMap, BTreeSet};

fn word(data: &[u8], offset: usize) -> Result<u16, String> {
    let bytes = data.get(offset..offset.saturating_add(2)).ok_or("truncated TrueType word")?;
    Ok(u16::from_be_bytes(bytes.try_into().map_err(|_| "invalid TrueType word")?))
}

fn dword(data: &[u8], offset: usize) -> Result<u32, String> {
    let bytes = data.get(offset..offset.saturating_add(4)).ok_or("truncated TrueType offset")?;
    Ok(u32::from_be_bytes(bytes.try_into().map_err(|_| "invalid TrueType offset")?))
}

fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, chunk| {
        let mut bytes = [0; 4];
        for (to, from) in bytes.iter_mut().zip(chunk) {
            *to = *from;
        }
        sum.wrapping_add(u32::from_be_bytes(bytes))
    })
}

fn put(data: &mut [u8], offset: usize, bytes: &[u8]) -> Result<(), String> {
    data.get_mut(offset..offset.saturating_add(bytes.len())).ok_or("invalid TrueType output offset")?.copy_from_slice(bytes);
    Ok(())
}

/// Keep requested outlines and their composite dependencies, dropping unused outlines and cmaps.
pub(crate) fn subset(source: &[u8], mappings: &[(char, u16)]) -> Result<Vec<u8>, String> {
    if source.len() > 16 << 20 || mappings.len() > 65_534 {
        return Err("TrueType subset exceeds its budget".into());
    }
    let mut tables = BTreeMap::<[u8; 4], Vec<u8>>::new();
    for index in 0..usize::from(word(source, 4)?) {
        let entry = 12 + index * 16;
        let tag: [u8; 4] = source.get(entry..entry + 4).ok_or("truncated TrueType directory")?.try_into().map_err(|_| "invalid table tag")?;
        let start = usize::try_from(dword(source, entry + 8)?).map_err(|_| "font table offset limit")?;
        let length = usize::try_from(dword(source, entry + 12)?).map_err(|_| "font table length limit")?;
        if matches!(&tag, b"head" | b"hhea" | b"maxp" | b"hmtx" | b"loca" | b"glyf" | b"name" | b"OS/2" | b"cvt " | b"fpgm" | b"prep" | b"gasp") {
            tables.insert(tag, source.get(start..start.saturating_add(length)).ok_or("truncated TrueType table")?.to_vec());
        }
    }
    let head = tables.get(b"head").ok_or("missing TrueType head")?;
    let long = match word(head, 50)? {
        0 => false,
        1 => true,
        _ => return Err("invalid TrueType loca format".into()),
    };
    let count = usize::from(word(tables.get(b"maxp").ok_or("missing TrueType maxp")?, 4)?);
    let loca = tables.get(b"loca").ok_or("missing TrueType loca")?;
    let glyf = tables.get(b"glyf").ok_or("missing TrueType glyf")?;
    let offsets = (0..=count)
        .map(|i| if long { dword(loca, i * 4).map(|v| v as usize) } else { word(loca, i * 2).map(|v| usize::from(v) * 2) })
        .collect::<Result<Vec<_>, _>>()?;
    let glyph = |id: u16| -> Result<&[u8], String> {
        let i = usize::from(id);
        if i >= count {
            return Err("subset glyph outside the font".into());
        }
        let start = *offsets.get(i).ok_or("missing glyph offset")?;
        let end = *offsets.get(i + 1).ok_or("missing glyph end")?;
        glyf.get(start..end).ok_or_else(|| "invalid glyph range".into())
    };
    let mut kept = BTreeSet::from([0u16]);
    let mut pending: Vec<u16> = mappings.iter().map(|(_, gid)| *gid).collect();
    pending.push(0);
    let mut visited = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        kept.insert(id);
        let data = glyph(id)?;
        if data.is_empty() || word(data, 0)? != 0xffff {
            continue;
        }
        let mut offset = 10usize;
        loop {
            let flags = word(data, offset)?;
            let child = word(data, offset + 2)?;
            kept.insert(child);
            pending.push(child);
            offset = offset.saturating_add(4 + if flags & 1 != 0 { 4 } else { 2 });
            offset = offset.saturating_add(if flags & 0x80 != 0 { 8 } else if flags & 0x40 != 0 { 4 } else if flags & 8 != 0 { 2 } else { 0 });
            if flags & 0x20 == 0 {
                break;
            }
        }
    }
    let mut outlines = Vec::new();
    let mut locations = Vec::new();
    for i in 0..count {
        locations.extend_from_slice(&u32::try_from(outlines.len()).map_err(|_| "subset offset limit")?.to_be_bytes());
        let id = u16::try_from(i).map_err(|_| "subset glyph limit")?;
        if kept.contains(&id) {
            outlines.extend_from_slice(glyph(id)?);
            while outlines.len() % 4 != 0 {
                outlines.push(0);
            }
        }
    }
    locations.extend_from_slice(&u32::try_from(outlines.len()).map_err(|_| "subset offset limit")?.to_be_bytes());
    tables.insert(*b"glyf", outlines);
    tables.insert(*b"loca", locations);
    let head = tables.get_mut(b"head").ok_or("missing TrueType head")?;
    put(head, 8, &[0; 4])?;
    put(head, 50, &1u16.to_be_bytes())?;
    // Format 12 keeps Unicode aliases separate while pointing at the original stable glyph IDs.
    let mut cmap = Vec::new();
    for v in [0u16, 1, 3, 10] {
        cmap.extend_from_slice(&v.to_be_bytes());
    }
    cmap.extend_from_slice(&12u32.to_be_bytes());
    for v in [12u16, 0] {
        cmap.extend_from_slice(&v.to_be_bytes());
    }
    let length = u32::try_from(16 + mappings.len() * 12).map_err(|_| "subset cmap limit")?;
    for v in [length, 0, mappings.len() as u32] {
        cmap.extend_from_slice(&v.to_be_bytes());
    }
    for &(ch, gid) in mappings {
        for v in [u32::from(ch), u32::from(ch), u32::from(gid)] {
            cmap.extend_from_slice(&v.to_be_bytes());
        }
    }
    tables.insert(*b"cmap", cmap);
    let mut post = vec![0; 32];
    put(&mut post, 0, &0x0003_0000u32.to_be_bytes())?;
    tables.insert(*b"post", post);
    let num = u16::try_from(tables.len()).map_err(|_| "subset table limit")?;
    let power = 1u16 << (15 - num.leading_zeros());
    let mut out = vec![0; 12 + tables.len() * 16];
    put(&mut out, 0, &0x0001_0000u32.to_be_bytes())?;
    for (offset, value) in [(4, num), (6, power * 16), (8, power.ilog2() as u16), (10, num * 16 - power * 16)] {
        put(&mut out, offset, &value.to_be_bytes())?;
    }
    let mut head_offset = None;
    for (i, (tag, data)) in tables.into_iter().enumerate() {
        let offset = out.len();
        if &tag == b"head" {
            head_offset = Some(offset);
        }
        let entry = 12 + i * 16;
        put(&mut out, entry, &tag)?;
        put(&mut out, entry + 4, &checksum(&data).to_be_bytes())?;
        put(&mut out, entry + 8, &u32::try_from(offset).map_err(|_| "subset offset limit")?.to_be_bytes())?;
        put(&mut out, entry + 12, &u32::try_from(data.len()).map_err(|_| "subset length limit")?.to_be_bytes())?;
        out.extend(data);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }
    let adjustment = 0xb1b0_afbau32.wrapping_sub(checksum(&out));
    put(&mut out, head_offset.ok_or("missing subset head")? + 8, &adjustment.to_be_bytes())?;
    Ok(out)
}
