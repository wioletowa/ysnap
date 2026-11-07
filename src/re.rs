// Регулярки для window-rule niri (match app-id=… title=…): ^ $ . * + ? | ( ) [...] [^...] и
// \d \w \s \. — то, что пишут в конфигах. Поиск где угодно в строке, как у niri (regex::is_match).
// Перебор с возвратом: строки короткие (app-id, заголовок окна).

#[derive(Debug, Clone)]
enum Node {
    Char(char),
    Any,
    Class(Vec<(char, char)>, bool),
    Start,
    End,
    Group(Vec<Vec<Node>>),
    Repeat(Box<Node>, usize, usize),
}

struct Parser<'a> {
    c: &'a [char],
    i: usize,
}

impl Parser<'_> {
    fn alt(&mut self) -> Vec<Vec<Node>> {
        let mut alts = vec![self.seq()];
        while self.c.get(self.i) == Some(&'|') {
            self.i += 1;
            alts.push(self.seq());
        }
        alts
    }

    fn seq(&mut self) -> Vec<Node> {
        let mut v = vec![];
        while let Some(&ch) = self.c.get(self.i) {
            if ch == '|' || ch == ')' {
                break;
            }
            self.i += 1;
            let atom = match ch {
                '.' => Node::Any,
                '^' => Node::Start,
                '$' => Node::End,
                '(' => {
                    // (?: … ) — просто группа
                    if self.c.get(self.i) == Some(&'?') && self.c.get(self.i + 1) == Some(&':') {
                        self.i += 2;
                    }
                    let g = self.alt();
                    if self.c.get(self.i) == Some(&')') {
                        self.i += 1;
                    }
                    Node::Group(g)
                }
                '[' => self.class(),
                '\\' => self.escape(),
                c => Node::Char(c),
            };
            let (lo, hi) = match self.c.get(self.i) {
                Some('*') => (0, usize::MAX),
                Some('+') => (1, usize::MAX),
                Some('?') => (0, 1),
                _ => {
                    v.push(atom);
                    continue;
                }
            };
            self.i += 1;
            // ленивость (*?) для «совпадает ли» не важна
            if self.c.get(self.i) == Some(&'?') {
                self.i += 1;
            }
            v.push(Node::Repeat(Box::new(atom), lo, hi));
        }
        v
    }

    fn escape(&mut self) -> Node {
        let Some(&e) = self.c.get(self.i) else { return Node::Char('\\') };
        self.i += 1;
        let r = |x: &[(char, char)], neg| Node::Class(x.to_vec(), neg);
        match e {
            'd' => r(&[('0', '9')], false),
            'D' => r(&[('0', '9')], true),
            'w' => r(&[('a', 'z'), ('A', 'Z'), ('0', '9'), ('_', '_')], false),
            'W' => r(&[('a', 'z'), ('A', 'Z'), ('0', '9'), ('_', '_')], true),
            's' => r(&[(' ', ' '), ('\t', '\t'), ('\n', '\n'), ('\r', '\r')], false),
            'S' => r(&[(' ', ' '), ('\t', '\t'), ('\n', '\n'), ('\r', '\r')], true),
            't' => Node::Char('\t'),
            'n' => Node::Char('\n'),
            c => Node::Char(c),
        }
    }

    fn class(&mut self) -> Node {
        let neg = self.c.get(self.i) == Some(&'^');
        if neg {
            self.i += 1;
        }
        let mut set = vec![];
        let mut first = true;
        while let Some(&ch) = self.c.get(self.i) {
            if ch == ']' && !first {
                self.i += 1;
                break;
            }
            first = false;
            self.i += 1;
            let lo = if ch == '\\' {
                match self.escape() {
                    Node::Class(r, false) => {
                        set.extend(r);
                        continue;
                    }
                    Node::Char(c) => c,
                    _ => continue,
                }
            } else {
                ch
            };
            if self.c.get(self.i) == Some(&'-') && self.c.get(self.i + 1).is_some_and(|c| *c != ']') {
                let hi = self.c[self.i + 1];
                self.i += 2;
                set.push((lo, hi));
            } else {
                set.push((lo, lo));
            }
        }
        Node::Class(set, neg)
    }
}

/// совпадение seq[k..] с s от позиции i; next — что должно совпасть после (продолжение)
fn m(seq: &[Node], s: &[char], i: usize, next: &mut dyn FnMut(usize) -> bool) -> bool {
    let Some(n) = seq.first() else { return next(i) };
    let rest = &seq[1..];
    match n {
        Node::Start => i == 0 && m(rest, s, i, next),
        Node::End => i == s.len() && m(rest, s, i, next),
        Node::Repeat(a, lo, hi) => rep(a, *lo, *hi, 0, rest, s, i, next),
        Node::Group(alts) => alts.iter().any(|alt| m(alt, s, i, &mut |j| m(rest, s, j, next))),
        _ => one(n, s.get(i)) && m(rest, s, i + 1, next),
    }
}

/// a повторён count раз, нужно от lo до hi; жадно, с возвратом
#[allow(clippy::too_many_arguments)]
fn rep(a: &Node, lo: usize, hi: usize, count: usize, rest: &[Node], s: &[char], i: usize, next: &mut dyn FnMut(usize) -> bool) -> bool {
    if count < hi && m(std::slice::from_ref(a), s, i, &mut |j| j > i && rep(a, lo, hi, count + 1, rest, s, j, next)) {
        return true;
    }
    count >= lo && m(rest, s, i, next)
}

fn one(n: &Node, c: Option<&char>) -> bool {
    let Some(&c) = c else { return false };
    match n {
        Node::Char(x) => *x == c,
        Node::Any => c != '\n',
        Node::Class(set, neg) => set.iter().any(|(a, b)| (*a..=*b).contains(&c)) != *neg,
        _ => false,
    }
}

/// есть ли совпадение pat где-нибудь в s
pub fn is_match(pat: &str, s: &str) -> bool {
    let pc: Vec<char> = pat.chars().collect();
    let tree = Node::Group(Parser { c: &pc, i: 0 }.alt());
    let sc: Vec<char> = s.chars().collect();
    let seq = [tree];
    (0..=sc.len()).any(|i| m(&seq, &sc, i, &mut |_| true))
}

#[cfg(test)]
mod tests {
    use super::is_match as re;

    #[test]
    fn niri_style_patterns() {
        assert!(re("^kitty$", "kitty") && !re("^kitty$", "kitty2"));
        assert!(re("zen$", "app.zen") && !re("zen$", "zenith"));
        assert!(re("^com\\.a", "com.ab") && !re("^com\\.a", "comXab"));
        assert!(re("^a.*z$", "abcz") && re("^(zen|firefox)$", "firefox") && !re("^(zen|firefox)$", "firefox-esr"));
        assert!(re("^org\\.gnome\\.[A-Z][a-z]+$", "org.gnome.Nautilus"));
        assert!(re("^Картинка в картинке$", "Картинка в картинке"));
        assert!(re("^[^ ]+ — Zen", "tab — Zen Browser") && !re("^[^ ]+ — Zen", "a b — Zen"));
        assert!(re("v\\d+", "app v12") && re("colou?r", "color") && re("", "anything"));
        assert!(re("^(?:steam_app_)\\d+$", "steam_app_570"));
    }
}
