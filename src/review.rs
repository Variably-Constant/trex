//! An interactive review of edits before they are written: each change put
//! in turn to whoever reviews them, an answer for a change's template
//! deciding every later change of that template without asking, and nothing
//! kept until the review has seen the last change. The trex command,
//! PowerShell and Python each supply only how a change is asked.

use crate::files::Edit;
use crate::token::TokenKind;

/// One input's changes awaiting review: its name, the text the edits are
/// over, and the edits in position order.
#[derive(Clone, Copy, Debug)]
pub struct Queued<'a> {
    /// The input's name, as a report names it.
    pub name: &'a str,
    /// The text the edits index.
    pub text: &'a [u8],
    /// The edits, in position order.
    pub edits: &'a [Edit],
}

/// One change put to the reviewer.
#[derive(Clone, Copy, Debug)]
pub struct Change<'a> {
    /// The input the change is in, by its place in the queue.
    pub input: usize,
    /// The input's name.
    pub name: &'a str,
    /// The input's text.
    pub text: &'a [u8],
    /// The change: the span it replaces and what it puts there.
    pub edit: &'a Edit,
    /// The change's template: the kinds of the tokens it replaces.
    pub template: &'a [TokenKind],
    /// How many changes after this one share its template, which an answer
    /// for the template decides with it.
    pub later: usize,
    /// Which change this is, from 1.
    pub index: usize,
    /// How many changes the review holds.
    pub total: usize,
}

/// What the reviewer answered for one change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Apply the change.
    Accept,
    /// Skip it.
    Skip,
    /// Apply this replacement in its place.
    Replace(Vec<u8>),
    /// Apply it and every later change of its template.
    AcceptTemplate,
    /// Skip it and every later change of its template.
    SkipTemplate,
    /// Apply it and every later change.
    AcceptAll,
    /// Stop, keeping no edit of any input.
    Quit,
}

/// A change an answer for its template passed over: its input, by its place
/// in the queue, and the span it would have replaced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Skipped {
    /// The input, by its place in the queue.
    pub input: usize,
    /// Where the change starts in the input's text.
    pub start: usize,
    /// Where it ends.
    pub end: usize,
}

/// What a finished review keeps: the edits accepted for each input, in the
/// queue's order, and the changes a template answer passed over.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reviewed {
    /// The edits accepted for each input, in the queue's order.
    pub kept: Vec<Vec<Edit>>,
    /// Each change an answer for its template skipped without asking.
    pub skipped: Vec<Skipped>,
}

/// Put each change of `queue` to `ask` in turn, input by input in position
/// order, and give back what the review keeps; `None` where the reviewer
/// quit, which keeps no edit of any input.
///
/// A change whose template an earlier answer decided is decided by it
/// without being asked; after an answer to apply every change, the rest are
/// kept without being asked.
///
/// # Errors
///
/// What `ask` returns, which ends the review with nothing kept.
pub fn review<E>(
    queue: &[Queued<'_>],
    mut ask: impl FnMut(&Change<'_>) -> Result<Answer, E>,
) -> Result<Option<Reviewed>, E> {
    let total: usize = queue.iter().map(|q| q.edits.len()).sum();
    // Every change's template, read once in the order they are offered, so
    // each question can say how many later changes an answer for its
    // template carries without lexing them again.
    let shapes: Vec<Vec<Vec<TokenKind>>> = queue
        .iter()
        .map(|q| q.edits.iter().map(|e| crate::templates::silhouette(q.text, e.start..e.end)).collect())
        .collect();
    let mut kept: Vec<Vec<Edit>> = queue.iter().map(|_| Vec::new()).collect();
    let mut answered: Vec<(&[TokenKind], bool)> = Vec::new();
    let mut skipped = Vec::new();
    let mut all = false;
    let mut index = 0usize;
    for (k, q) in queue.iter().enumerate() {
        for (e, edit) in q.edits.iter().enumerate() {
            index += 1;
            if all {
                kept[k].push(edit.clone());
                continue;
            }
            let shape = shapes[k][e].as_slice();
            if let Some(&(_, accepted)) = answered.iter().find(|(s, _)| *s == shape) {
                if accepted {
                    kept[k].push(edit.clone());
                } else {
                    skipped.push(Skipped { input: k, start: edit.start, end: edit.end });
                }
                continue;
            }
            let later = shapes
                .iter()
                .enumerate()
                .flat_map(|(j, per_input)| per_input.iter().enumerate().map(move |(i, s)| (j, i, s)))
                .filter(|&(j, i, s)| (j > k || (j == k && i > e)) && s.as_slice() == shape)
                .count();
            let change =
                Change { input: k, name: q.name, text: q.text, edit, template: shape, later, index, total };
            match ask(&change)? {
                Answer::Accept => kept[k].push(edit.clone()),
                Answer::Skip => {}
                Answer::Replace(replacement) => {
                    kept[k].push(Edit { start: edit.start, end: edit.end, replacement });
                }
                Answer::AcceptTemplate => {
                    kept[k].push(edit.clone());
                    answered.push((shape, true));
                }
                Answer::SkipTemplate => {
                    answered.push((shape, false));
                    skipped.push(Skipped { input: k, start: edit.start, end: edit.end });
                }
                Answer::AcceptAll => {
                    kept[k].push(edit.clone());
                    all = true;
                }
                Answer::Quit => return Ok(None),
            }
        }
    }
    Ok(Some(Reviewed { kept, skipped }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(text: &[u8], from: &str, to: &str) -> Edit {
        let start = text.windows(from.len()).position(|w| w == from.as_bytes()).expect("the text holds it");
        Edit { start, end: start + from.len(), replacement: to.as_bytes().to_vec() }
    }

    /// Two inputs: two number changes and a word change, then one more of
    /// each, so a template answer has later changes to decide.
    fn queued<'a>(a: &'a [u8], b: &'a [u8], ea: &'a [Edit], eb: &'a [Edit]) -> Vec<Queued<'a>> {
        vec![Queued { name: "a", text: a, edits: ea }, Queued { name: "b", text: b, edits: eb }]
    }

    #[test]
    fn each_answer_keeps_what_it_says() {
        let a = b"x 1 y 2 word";
        let b = b"z 3 more";
        let ea = [edit(a, "1", "one"), edit(a, "2", "two"), edit(a, "word", "WORD")];
        let eb = [edit(b, "3", "three"), edit(b, "more", "MORE")];
        let q = queued(a, b, &ea, &eb);
        let mut asked = Vec::new();
        let answers = [Answer::Accept, Answer::Replace(b"deux".to_vec()), Answer::Skip, Answer::Skip, Answer::Accept];
        let mut next = answers.iter();
        let reviewed = review(&q, |c| -> Result<Answer, ()> {
            asked.push((c.name.to_string(), c.index, c.total, c.later));
            Ok(next.next().expect("an answer for each change").clone())
        })
        .expect("no error")
        .expect("not quit");
        // The first number change has two later number changes, the word
        // change one later word change.
        assert_eq!(
            asked,
            [
                ("a".to_string(), 1, 5, 2),
                ("a".to_string(), 2, 5, 1),
                ("a".to_string(), 3, 5, 1),
                ("b".to_string(), 4, 5, 0),
                ("b".to_string(), 5, 5, 0),
            ]
        );
        assert_eq!(reviewed.kept[0], [ea[0].clone(), edit(a, "2", "deux")]);
        assert_eq!(reviewed.kept[1], [eb[1].clone()]);
        assert!(reviewed.skipped.is_empty());
    }

    #[test]
    fn a_template_answer_decides_the_later_changes_of_its_template() {
        let a = b"x 1 y 2 word";
        let b = b"z 3 more";
        let ea = [edit(a, "1", "one"), edit(a, "2", "two"), edit(a, "word", "WORD")];
        let eb = [edit(b, "3", "three"), edit(b, "more", "MORE")];
        let q = queued(a, b, &ea, &eb);
        let mut asked = 0;
        let reviewed = review(&q, |c| -> Result<Answer, ()> {
            asked += 1;
            Ok(if c.edit.replacement == b"one" { Answer::SkipTemplate } else { Answer::AcceptTemplate })
        })
        .expect("no error")
        .expect("not quit");
        // Asked once for the numbers and once for the words.
        assert_eq!(asked, 2);
        assert_eq!(reviewed.kept, [vec![ea[2].clone()], vec![eb[1].clone()]]);
        assert_eq!(
            reviewed.skipped,
            [
                Skipped { input: 0, start: ea[0].start, end: ea[0].end },
                Skipped { input: 0, start: ea[1].start, end: ea[1].end },
                Skipped { input: 1, start: eb[0].start, end: eb[0].end },
            ]
        );
    }

    #[test]
    fn all_keeps_the_rest_and_quit_keeps_nothing() {
        let a = b"x 1 y 2 word";
        let b = b"z 3 more";
        let ea = [edit(a, "1", "one"), edit(a, "2", "two"), edit(a, "word", "WORD")];
        let eb = [edit(b, "3", "three"), edit(b, "more", "MORE")];
        let q = queued(a, b, &ea, &eb);
        let mut asked = 0;
        let reviewed = review(&q, |_| -> Result<Answer, ()> {
            asked += 1;
            Ok(if asked == 1 { Answer::Skip } else { Answer::AcceptAll })
        })
        .expect("no error")
        .expect("not quit");
        assert_eq!(asked, 2);
        assert_eq!(reviewed.kept, [vec![ea[1].clone(), ea[2].clone()], eb.to_vec()]);

        let quit = review(&q, |c| -> Result<Answer, ()> { Ok(if c.index == 3 { Answer::Quit } else { Answer::Accept }) });
        assert_eq!(quit, Ok(None));
        let failed = review(&q, |_| -> Result<Answer, &str> { Err("no answer") });
        assert_eq!(failed, Err("no answer"));
    }
}
