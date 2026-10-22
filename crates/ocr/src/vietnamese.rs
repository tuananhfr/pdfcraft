//! EasyOCR latin_g2 grayscale line recognition and CTC alignment (native only).

use image::{GrayImage, imageops::FilterType};
use rten_tensor::{Tensor, prelude::*};
use unicode_normalization::UnicodeNormalization;

use super::{OcrError, Word};

// Class order from EasyOCR 363afb184047ce452e436f4224f3098422df872e, config.py latin_g2.
pub(super) const ALPHABET: &str = " !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~ªÀÁÂÃÄÅÆÇÈÉÊËÌÍÎÏÐÑÒÓÔÕÖØÙÚÛÜÝÞßàáâãäåæçèéêëìíîïðñòóôõöøùúûüýþÿĀāĂăĄąĆćČčĎďĐđĒēĖėĘęĚěĞğĨĩĪīĮįİıĶķĹĺĻļĽľŁłŃńŅņŇňŒœŔŕŘřŚśŞşŠšŤťŨũŪūŮůŲųŸŹźŻżŽžƏƠơƯưȘșȚțə̇ḌḍḶḷṀṁṂṃṄṅṆṇṬṭẠạẢảẤấẦầẨẩẪẫẬậẮắẰằẲẳẴẵẶặẸẹẺẻẼẽẾếỀềỂểỄễỆệỈỉỊịỌọỎỏỐốỒồỔổỖỗỘộỚớỜờỞởỠỡỢợỤụỦủỨứỪừỬửỮữỰựỲỳỴỵỶỷỸỹ€";

fn failed(message: impl ToString) -> OcrError {
    OcrError::Recognize(message.to_string())
}

pub(super) struct Recognizer {
    model: rten::Model,
}

impl Recognizer {
    pub fn load(path: &std::path::Path) -> Result<Self, OcrError> {
        let model = rten::Model::load_file(path).map_err(|e| OcrError::Load(path.display().to_string(), e.to_string()))?;
        Ok(Self { model })
    }

    pub fn read(&self, pixels: &[u8], width: u32, height: u32, rect: [f32; 4]) -> Result<Vec<Word>, OcrError> {
        if width == 0 || height == 0 || !rect.iter().all(|v| v.is_finite()) || rect[2] <= rect[0] || rect[3] <= rect[1] {
            return Err(failed("invalid recognition rectangle"));
        }
        let count = usize::try_from(u64::from(width) * u64::from(height)).map_err(failed)?;
        let channels = pixels.len().checked_div(count).filter(|c| matches!(c, 1 | 3 | 4)).ok_or_else(|| failed("invalid pixels"))?;
        let pad = ((rect[3] - rect[1]) * 0.2).ceil().clamp(2.0, 64.0);
        let left = (rect[0] - pad).floor().max(0.0).min(width as f32) as u32;
        let top = (rect[1] - pad).floor().max(0.0).min(height as f32) as u32;
        let right = (rect[2] + pad).ceil().max(0.0).min(width as f32) as u32;
        let bottom = (rect[3] + pad).ceil().max(0.0).min(height as f32) as u32;
        let (w, h) = (right.saturating_sub(left), bottom.saturating_sub(top));
        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 16_000_000 {
            return Err(failed("recognition crop is empty or exceeds 16 million pixels"));
        }
        let normalized_width = (f64::from(w) * 64.0 / f64::from(h)).round().max(16.0);
        if normalized_width > 4096.0 {
            return Err(failed("text line exceeds recognition limit (4096 normalized pixels); use shorter lines"));
        }
        let mut gray = Vec::new();
        for y in top..bottom {
            for x in left..right {
                let offset = (u64::from(y) * u64::from(width) + u64::from(x)) * channels as u64;
                let offset = usize::try_from(offset).map_err(failed)?;
                let px = pixels.get(offset..offset.saturating_add(channels)).ok_or_else(|| failed("pixel buffer is too short"))?;
                let value = match px {
                    [v] => *v,
                    [r, g, b] | [r, g, b, _] => ((u32::from(*r) * 299 + u32::from(*g) * 587 + u32::from(*b) * 114 + 500) / 1000) as u8,
                    _ => return Err(failed("invalid channels")),
                };
                gray.push(value);
            }
        }
        let image = GrayImage::from_raw(w, h, gray).ok_or_else(|| failed("invalid crop dimensions"))?;
        let resized = image::imageops::resize(&image, normalized_width as u32, 64, FilterType::CatmullRom);
        let tensor = Tensor::from_data(
            &[1, 1, 64, normalized_width as usize][..],
            resized.into_raw().into_iter().map(|v| f32::from(v) / 127.5 - 1.0).collect::<Vec<_>>(),
        );
        let output = self.model.run_one(tensor.view().into(), None).map_err(failed)?;
        let output: Tensor<f32> = output.try_into().map_err(failed)?;
        let classes = ALPHABET.chars().count() + 1;
        let [1, time, nclasses] = output.shape() else { return Err(failed("expected recognition logits [1, time, classes]")) };
        if *time == 0 || *nclasses != classes {
            return Err(failed("Vietnamese model output does not match latin_g2 alphabet"));
        }
        let data = output.data().ok_or_else(|| failed("non-contiguous recognition output"))?;
        let decoded = decode(data, classes)?;
        let mut words = Vec::new();
        let mut text = String::new();
        let mut start = 0;
        let mut end = 0;
        let make_word = |text: &str, start: usize, end: usize| Word {
            text: text.nfc().collect(),
            rect: [
                (left as f32 + start as f32 / *time as f32 * w as f32).max(rect[0]),
                rect[1],
                (left as f32 + (end + 1) as f32 / *time as f32 * w as f32).min(rect[2]),
                rect[3],
            ],
        };
        for (c, first, last) in decoded {
            if c.is_whitespace() {
                if !text.is_empty() {
                    words.push(make_word(&text, start, end));
                    text.clear();
                }
            } else {
                if text.is_empty() {
                    start = first;
                }
                text.push(c);
                end = last;
            }
        }
        if !text.is_empty() {
            words.push(make_word(&text, start, end));
        }
        Ok(words.into_iter().filter(|w| w.rect[2] > w.rect[0]).collect())
    }
}

fn decode(logits: &[f32], classes: usize) -> Result<Vec<(char, usize, usize)>, OcrError> {
    if classes != ALPHABET.chars().count() + 1 || !logits.len().is_multiple_of(classes) {
        return Err(failed("invalid CTC dimensions"));
    }
    let alphabet: Vec<char> = std::iter::once('\0').chain(ALPHABET.chars()).collect();
    let mut previous = 0;
    let mut result: Vec<(char, usize, usize)> = Vec::new();
    for (time, row) in logits.chunks_exact(classes).enumerate() {
        if row.iter().any(|v| !v.is_finite()) {
            return Err(failed("non-finite recognition logits"));
        }
        let class = row.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map_or(0, |(i, _)| i);
        if class != 0 {
            if class != previous {
                result.push((*alphabet.get(class).ok_or_else(|| failed("invalid CTC class"))?, time, time));
            } else if let Some(last) = result.last_mut() {
                last.2 = time;
            }
        }
        previous = class;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vietnamese_alphabet_covers_every_tone_and_case() {
        for base in "aăâeêioôơuưyAĂÂEÊIOÔƠUƯY".chars() {
            for tone in ["", "\u{301}", "\u{300}", "\u{309}", "\u{303}", "\u{323}"] {
                for c in format!("{base}{tone}").nfc() {
                    assert!(ALPHABET.contains(c), "missing {c}");
                }
            }
        }
        assert!(ALPHABET.contains('đ') && ALPHABET.contains('Đ'));
    }

    #[test]
    fn ctc_preserves_case_blank_separated_repeats_and_alignment() {
        let classes = ALPHABET.chars().count() + 1;
        let class = |c| ALPHABET.chars().position(|v| v == c).unwrap() + 1;
        let sequence = [class('C'), class('C'), 0, class('c'), 0, class('c'), class('ộ')];
        let mut logits = vec![-1.0; classes * sequence.len()];
        for (time, index) in sequence.into_iter().enumerate() {
            logits[time * classes + index] = 1.0;
        }
        assert_eq!(decode(&logits, classes).unwrap(), [('C', 0, 1), ('c', 3, 3), ('c', 5, 5), ('ộ', 6, 6)]);
        assert!(decode(&[f32::NAN; 2], 0).is_err());
    }
}
