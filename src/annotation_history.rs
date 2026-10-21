//! Bounded edit commands; never snapshots of all annotations or image pixels.
use crate::annotations::Annotation;

const LIMIT: usize = 64;

#[derive(Clone)]
enum Command {
    Insert {
        index: usize,
        value: Annotation,
    },
    Remove {
        index: usize,
        value: Annotation,
    },
    Edit {
        index: usize,
        before: Annotation,
        after: Annotation,
    },
}

struct Applied {
    selected: Option<usize>,
}

impl Command {
    fn apply(&self, annotations: &mut Vec<Annotation>, forward: bool) -> Option<Applied> {
        match self {
            Self::Insert { index, value } if forward => insert(annotations, *index, value),
            Self::Insert { index, value } => remove(annotations, *index, value),
            Self::Remove { index, value } if forward => remove(annotations, *index, value),
            Self::Remove { index, value } => insert(annotations, *index, value),
            Self::Edit {
                index,
                before,
                after,
            } => {
                let (expected, replacement) = if forward {
                    (before, after)
                } else {
                    (after, before)
                };
                if annotations.get(*index) != Some(expected) {
                    return None;
                }
                annotations[*index] = replacement.clone();
                Some(Applied {
                    selected: Some(*index),
                })
            }
        }
    }
}

fn insert(annotations: &mut Vec<Annotation>, index: usize, value: &Annotation) -> Option<Applied> {
    if index > annotations.len() || annotations.len() >= 10000 {
        return None;
    }
    annotations.insert(index, value.clone());
    Some(Applied {
        selected: Some(index),
    })
}

fn remove(annotations: &mut Vec<Annotation>, index: usize, value: &Annotation) -> Option<Applied> {
    if annotations.get(index) != Some(value) {
        return None;
    }
    annotations.remove(index);
    Some(Applied { selected: None })
}

#[derive(Default)]
pub(crate) struct History {
    undo: Vec<Command>,
    redo: Vec<Command>,
    edit_group: Option<usize>,
}

impl History {
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn end_group(&mut self) {
        self.edit_group = None;
    }

    fn push(&mut self, command: Command) {
        self.redo.clear();
        self.edit_group = None;
        self.undo.push(command);
        if self.undo.len() > LIMIT {
            self.undo.remove(0);
        }
    }

    pub fn inserted(&mut self, index: usize, value: &Annotation) {
        self.push(Command::Insert {
            index,
            value: value.clone(),
        });
    }
    pub fn removed(&mut self, index: usize, value: &Annotation) {
        self.push(Command::Remove {
            index,
            value: value.clone(),
        });
    }
    pub fn edited(&mut self, index: usize, before: Annotation, after: Annotation) {
        if before == after {
            return;
        }
        self.redo.clear();
        if self.edit_group == Some(index) {
            if let Some(Command::Edit {
                after: previous, ..
            }) = self.undo.last_mut()
            {
                *previous = after;
                return;
            }
        }
        self.push(Command::Edit {
            index,
            before,
            after,
        });
        self.edit_group = Some(index);
    }

    pub fn undo(&mut self, annotations: &mut Vec<Annotation>) -> Option<usize> {
        self.end_group();
        let command = self.undo.pop()?;
        if let Some(applied) = command.apply(annotations, false) {
            self.redo.push(command);
            applied.selected
        } else {
            // Refuse a stale command rather than mutate a different annotation.
            *self = Self::default();
            None
        }
    }
    pub fn redo(&mut self, annotations: &mut Vec<Annotation>) -> Option<usize> {
        self.end_group();
        let command = self.redo.pop()?;
        if let Some(applied) = command.apply(annotations, true) {
            self.undo.push(command);
            applied.selected
        } else {
            *self = Self::default();
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn annotation() -> Annotation {
        Annotation {
            appearance: Default::default(),
            tool: crate::annotations::Tool::Line,
            points: vec![[0.0, 0.0], [10.0, 20.0]],
            plane: 2,
            note: String::new(),
            origin: Some(7),
        }
    }
    #[test]
    fn insert_edit_delete_undo_redo_preserves_geometry_origin_and_branching() {
        let mut h = History::default();
        let mut values = vec![annotation()];
        h.inserted(0, &values[0]);
        let original = values[0].clone();
        values[0].note = "A".into();
        h.edited(0, original.clone(), values[0].clone());
        let intermediate = values[0].clone();
        values[0].note = "AB".into();
        h.edited(0, intermediate, values[0].clone());
        let removed = values.remove(0);
        h.removed(0, &removed);
        assert_eq!(h.undo(&mut values), Some(0));
        assert_eq!(values[0].note, "AB");
        h.undo(&mut values);
        assert!(values[0] == original);
        h.undo(&mut values);
        assert!(values.is_empty());
        h.redo(&mut values);
        h.redo(&mut values);
        h.redo(&mut values);
        assert!(values.is_empty());
        h.undo(&mut values);
        let before = values[0].clone();
        values[0].appearance.title = "New branch".into();
        h.edited(0, before, values[0].clone());
        assert!(!h.can_redo());
        assert_eq!(values[0].origin, Some(7));
    }
    #[test]
    fn history_is_bounded_and_stale_indices_do_not_delete_unrelated_values() {
        let mut h = History::default();
        let mut values = Vec::new();
        for i in 0..100 {
            values.push(annotation());
            h.inserted(i, &values[i]);
        }
        assert_eq!(h.undo.len(), LIMIT);
        values.last_mut().unwrap().note = "External change".into();
        h.undo(&mut values);
        assert_eq!(values.len(), 100);
        assert!(!h.can_undo());
    }
}
