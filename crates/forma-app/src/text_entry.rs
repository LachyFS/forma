//! Small single-line editors with an end caret and optional whole-text selection.
//! Validate an entire input before mutation so rejected paste never loses text.
use crate::app::Field;

pub(crate) const MAX_BYTES: usize = 128;

#[derive(Clone, Copy)]
pub(crate) enum EntryKind {
    Name,
    Expression,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum InputError {
    UnsupportedCharacter,
    TooLong,
}

pub(crate) fn insert(
    text: &mut String,
    selected: &mut bool,
    input: &str,
    kind: EntryKind,
) -> Result<bool, InputError> {
    if input.is_empty() {
        return Ok(false);
    }
    if !input.chars().all(|c| match kind {
        EntryKind::Name => !c.is_control(),
        EntryKind::Expression => crate::transform::numeric_input_char(c),
    }) {
        return Err(InputError::UnsupportedCharacter);
    }
    let retained = if *selected { 0 } else { text.len() };
    if input.len() > MAX_BYTES.saturating_sub(retained) {
        return Err(InputError::TooLong);
    }
    if *selected {
        text.clear();
    }
    text.push_str(input);
    *selected = false;
    Ok(true)
}

#[derive(Clone, Copy)]
pub(crate) enum Erase {
    Backspace,
    PreviousWord,
    ToStart,
    Delete,
}

pub(crate) fn erase(text: &mut String, selected: &mut bool, how: Erase) -> bool {
    let before = text.len();
    if *selected || matches!(how, Erase::ToStart) {
        text.clear();
    } else {
        match how {
            Erase::Backspace => {
                text.pop();
            }
            Erase::PreviousWord => {
                let trimmed = text.trim_end_matches(char::is_whitespace).len();
                text.truncate(trimmed);
                if let Some(last) = text.chars().next_back() {
                    let word = last.is_alphanumeric() || last == '_';
                    let start = text
                        .char_indices()
                        .rev()
                        .take_while(|(_, c)| {
                            !c.is_whitespace() && (c.is_alphanumeric() || *c == '_') == word
                        })
                        .last()
                        .map(|(index, _)| index)
                        .unwrap_or(text.len());
                    text.truncate(start);
                }
            }
            // The caret is at the end, so forward delete only removes a selection.
            Erase::Delete | Erase::ToStart => {}
        }
    }
    *selected = false;
    text.len() != before
}

/// Tab stays in the current inspector section and skips controls hidden by the
/// current shader, texture configuration or viewport mode.
pub(crate) fn next_field(
    current: Field,
    backwards: bool,
    available: impl Fn(Field) -> bool,
) -> Field {
    use Field::*;
    let group: &[Field] = match current {
        Name | Translation(_) | Rotation(_) | Scale(_) | Dimensions(_) => &[
            Name,
            Translation(0),
            Translation(1),
            Translation(2),
            Rotation(0),
            Rotation(1),
            Rotation(2),
            Scale(0),
            Scale(1),
            Scale(2),
            Dimensions(0),
            Dimensions(1),
            Dimensions(2),
        ],
        Color(_) | Roughness | Metallic | Ior | Emission | TextureScale(_) | TextureOffset(_)
        | NormalStrength => &[
            Color(0),
            Color(1),
            Color(2),
            Roughness,
            Metallic,
            Ior,
            Emission,
            TextureScale(0),
            TextureScale(1),
            TextureOffset(0),
            TextureOffset(1),
            NormalStrength,
        ],
        Samples | Bounces | Exposure | WorldStrength | DenoiseStart => {
            &[Samples, Bounces, Exposure, WorldStrength, DenoiseStart]
        }
        PreviewRotation | PreviewStrength | PreviewOpacity | PreviewBlur => &[
            PreviewRotation,
            PreviewStrength,
            PreviewOpacity,
            PreviewBlur,
        ],
    };
    let Some(index) = group.iter().position(|field| *field == current) else {
        return current;
    };
    (1..=group.len())
        .map(|step| {
            let offset = if backwards { group.len() - step } else { step };
            group[(index + offset) % group.len()]
        })
        .find(|field| available(*field))
        .unwrap_or(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_replaces_all_or_appends_without_losing_rejected_input() {
        let mut text = "25mm".to_owned();
        let mut selected = true;
        assert_eq!(
            insert(&mut text, &mut selected, "1/8in", EntryKind::Expression),
            Ok(true)
        );
        assert_eq!(text, "1/8in");
        assert!(!selected);
        assert_eq!(
            insert(&mut text, &mut selected, "+2mm", EntryKind::Expression),
            Ok(true)
        );
        assert_eq!(text, "1/8in+2mm");
        for input in ["1;2", "2mm\n3mm", "4xyz"] {
            selected = true;
            assert_eq!(
                insert(&mut text, &mut selected, input, EntryKind::Expression),
                Err(InputError::UnsupportedCharacter)
            );
            assert_eq!(text, "1/8in+2mm");
            assert!(selected);
        }
    }

    #[test]
    fn oversized_input_is_atomic_and_limits_are_utf8_bytes() {
        let mut text = "unchanged".to_owned();
        let mut selected = true;
        assert_eq!(
            insert(&mut text, &mut selected, &"é".repeat(65), EntryKind::Name),
            Err(InputError::TooLong)
        );
        assert_eq!(text, "unchanged");
        assert!(selected);
        assert_eq!(
            insert(&mut text, &mut selected, &"é".repeat(64), EntryKind::Name),
            Ok(true)
        );
        assert_eq!(text.len(), MAX_BYTES);
        assert_eq!(
            insert(&mut text, &mut selected, "a", EntryKind::Name),
            Err(InputError::TooLong)
        );
        assert_eq!(text.len(), MAX_BYTES);
    }

    #[test]
    fn empty_paste_preserves_text_and_whole_text_selection() {
        let mut text = "1/8in".to_owned();
        let mut selected = true;
        assert_eq!(
            insert(&mut text, &mut selected, "", EntryKind::Expression),
            Ok(false)
        );
        assert_eq!(text, "1/8in");
        assert!(selected);
    }

    #[test]
    fn deletion_respects_whole_selection_end_caret_and_unicode_boundaries() {
        let mut text = "pièce".to_owned();
        let mut selected = false;
        assert!(!erase(&mut text, &mut selected, Erase::Delete));
        assert!(erase(&mut text, &mut selected, Erase::Backspace));
        assert_eq!(text, "pièc");
        selected = true;
        assert!(erase(&mut text, &mut selected, Erase::Delete));
        assert!(text.is_empty());
        assert!(!selected);
        text = "1m + 25mm  ".into();
        erase(&mut text, &mut selected, Erase::PreviousWord);
        assert_eq!(text, "1m + ");
        erase(&mut text, &mut selected, Erase::ToStart);
        assert!(text.is_empty());
    }

    #[test]
    fn tab_cycles_transform_axes_and_skips_hidden_surface_controls() {
        assert_eq!(
            next_field(Field::Name, false, |_| true),
            Field::Translation(0)
        );
        assert_eq!(
            next_field(Field::Translation(2), false, |_| true),
            Field::Rotation(0)
        );
        assert_eq!(
            next_field(Field::Dimensions(2), false, |_| true),
            Field::Name
        );
        assert_eq!(
            next_field(Field::Name, true, |_| true),
            Field::Dimensions(2)
        );
        assert_eq!(
            next_field(Field::Roughness, false, |f| f != Field::Metallic),
            Field::Ior
        );
        assert_eq!(
            next_field(Field::Color(0), true, |f| f != Field::NormalStrength),
            Field::TextureOffset(1)
        );
        assert_eq!(
            next_field(Field::Exposure, false, |f| f == Field::Exposure),
            Field::Exposure
        );
    }
}
