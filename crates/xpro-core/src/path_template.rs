//! Path templates, like `{image}.png`, which name the files output nodes write after the image
//! files a graph's inputs are bound to.

use crate::eval::{Input, Inputs};
use anyhow::{Context as _, anyhow, bail};
use std::{ffi::OsString, path::PathBuf};

/// Expands a path template, such as `{image}.png`, where `{name}` stands for the file stem of the
/// image file bound to the graph input called `name`. `{{` and `}}` stand for literal braces.
pub fn expand_path_template(template: &str, inputs: &Inputs) -> anyhow::Result<PathBuf> {
    let mut path = OsString::new();
    let mut rest = template;
    while let Some(i) = rest.find(['{', '}']) {
        path.push(&rest[..i]);
        let (brace, after) = rest[i..].split_at(1);
        if after.starts_with(brace) {
            path.push(brace);
            rest = &after[1..];
            continue;
        }
        if brace == "}" {
            bail!("Unmatched \"}}\" in the path {template:?}; write \"}}}}\" for a literal \"}}\"");
        }
        let end = after
            .find('}')
            .ok_or_else(|| anyhow!("Unmatched \"{{\" in the path {template:?}"))?;
        let name = &after[..end];
        let Input::Path(input_path) = inputs.get(name)? else {
            bail!("The graph input \"{name}\" isn't a file, so it has no name to use in a path");
        };
        let stem = input_path
            .file_stem()
            .with_context(|| format!("{} has no file name", input_path.display()))?;
        path.push(stem);
        rest = &after[end + 1..];
    }
    path.push(rest);
    if path.is_empty() {
        bail!("The path is empty");
    }
    Ok(PathBuf::from(path))
}

/// Writes `text` as a path template that expands to exactly `text`, by doubling its braces.
#[must_use]
pub fn escape_path_template(text: &str) -> String {
    text.replace('{', "{{").replace('}', "}}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use hegel::{TestCase, generators as gs};

    /// Text with no braces.
    fn gen_plain_text() -> impl gs::PrintableGenerator<String> {
        gs::text().exclude_characters("{}")
    }

    fn bound_to(name: &str, path: &str) -> Inputs {
        Inputs::new().bind(name, Input::Path(PathBuf::from(path)))
    }

    #[hegel::test(test_cases = 10_000)]
    fn escaped_text_expands_to_itself(tc: TestCase) {
        let text = tc.draw(gs::text().min_size(1));
        let template = escape_path_template(&text);
        let path = expand_path_template(&template, &Inputs::new()).unwrap();
        assert_eq!(path, PathBuf::from(text));
    }

    #[hegel::test(test_cases = 10_000)]
    fn a_placeholder_expands_to_its_inputs_file_stem(tc: TestCase) {
        let before = tc.draw(gen_plain_text());
        let after = tc.draw(gen_plain_text());
        let name = tc.draw(gen_plain_text());
        let stem = tc.draw(gs::text().alphabet("abcXYZ_-0123 ").min_size(1));
        let extension = tc.draw(gs::sampled_from(vec!["", ".NEF", ".dng", ".tar.gz"]));
        let file = format!("{stem}{extension}");
        tc.assume(PathBuf::from(&file).file_stem().is_some());
        let expected_stem = PathBuf::from(&file).file_stem().unwrap().to_owned();
        let inputs = bound_to(&name, &format!("some/dir/{file}"));
        let path = expand_path_template(&format!("{before}{{{name}}}{after}"), &inputs).unwrap();
        let mut expected = OsString::from(&before);
        expected.push(&expected_stem);
        expected.push(&after);
        assert_eq!(path.into_os_string(), expected);
    }

    #[hegel::test(test_cases = 4_000)]
    fn placeholders_for_unbound_inputs_are_rejected(tc: TestCase) {
        let name = tc.draw(gen_plain_text());
        let other = tc.draw(gen_plain_text());
        tc.assume(name != other);
        let inputs = bound_to(&other, "DSC_4362.NEF");
        assert!(expand_path_template(&format!("{{{name}}}.png"), &inputs).is_err());
    }

    #[hegel::test(test_cases = 10_000)]
    fn unmatched_braces_are_rejected(tc: TestCase) {
        let before = tc.draw(gen_plain_text());
        let after = tc.draw(gen_plain_text());
        let brace = tc.draw(gs::sampled_from(vec!["{", "}"]));
        let inputs = bound_to(&after, "DSC_4362.NEF");
        let template = format!("{before}{brace}{after}");
        assert!(
            expand_path_template(&template, &inputs).is_err(),
            "{template}"
        );
    }

    #[test]
    fn empty_paths_are_rejected() {
        assert!(expand_path_template("", &Inputs::new()).is_err());
    }
}
