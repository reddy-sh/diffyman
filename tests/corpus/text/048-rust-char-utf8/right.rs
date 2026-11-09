pub fn classify(c: char) -> Kind {
    match c {
        'a'..='f' => Kind::Lower,
        'A'..='Z' => Kind::Upper,
        '0'..='7' => Kind::Digit,
        'ä' | 'ő' | 'ü' => Kind::Umlaut,
        _ => Kind::Other,
    }
}
