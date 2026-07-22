use orbit_vm::NativeContext;

pub(crate) fn search(
    context: &NativeContext<'_>,
    name: &[u8],
    path: &[u8],
    separator: &[u8],
    replacement: &[u8],
) -> Result<Vec<u8>, Vec<u8>> {
    let transformed_name = replace_all(name, separator, replacement);
    let mut message = Vec::new();

    for template in path.split(|byte| *byte == b';') {
        let candidate = replace_all(template, b"?", &transformed_name);

        if context.file_exists(&candidate) {
            return Ok(candidate);
        }

        if !message.is_empty() {
            message.extend_from_slice(b"\n\t");
        }

        message.extend_from_slice(b"no file '");
        message.extend_from_slice(&candidate);
        message.push(b'\'');
    }

    Err(message)
}

fn replace_all(input: &[u8], needle: &[u8], replacement: &[u8]) -> Vec<u8> {
    if needle.is_empty() {
        return input.to_vec();
    }

    let mut result = Vec::new();
    let mut position = 0;

    while let Some(offset) = input[position..]
        .windows(needle.len())
        .position(|window| window == needle)
    {
        let start = position + offset;

        result.extend_from_slice(&input[position..start]);
        result.extend_from_slice(replacement);

        position = start + needle.len();
    }

    result.extend_from_slice(&input[position..]);
    result
}
