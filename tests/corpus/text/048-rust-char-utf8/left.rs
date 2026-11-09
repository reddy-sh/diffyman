pub fn classify(c: char) -> Kind {
    match c {
        'a'..='z' => Kind::Lower,
        'A'..='Z' => Kind::Upper,
        '0'..='9' => Kind::Digit,
        'ä' | 'ö' | 'ü' => Kind::Umlaut,
        _ => Kind::Other,
    }
}
