

#[derive(Debug, Clone)]
pub struct Pos {
    pub col: usize,
    pub row: usize,
    pub offset: usize,
}

impl Pos {
    pub fn new(col: usize, row: usize, offset: usize) -> Self {
        Self { col, row, offset }
    }
}


