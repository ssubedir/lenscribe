/// One insertion, deletion, substitution, or adjacent transposition, using Unicode characters.
pub(super) fn within_one_edit(left: &[char], right: &[char]) -> bool {
    if left.len().abs_diff(right.len()) > 1 {
        return false;
    }
    let first = left.iter().zip(right).position(|(a, b)| a != b);
    let Some(index) = first else {
        return true;
    };
    match left.len().cmp(&right.len()) {
        std::cmp::Ordering::Less => left[index..] == right[index + 1..],
        std::cmp::Ordering::Greater => left[index + 1..] == right[index..],
        std::cmp::Ordering::Equal => {
            left[index + 1..] == right[index + 1..]
                || (index + 1 < left.len()
                    && left[index] == right[index + 1]
                    && left[index + 1] == right[index]
                    && left[index + 2..] == right[index + 2..])
        }
    }
}
