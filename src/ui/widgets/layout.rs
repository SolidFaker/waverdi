use ratatui::layout::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Constraint {
    Fixed(u16),
    Ratio(u32, u32),
    Fill,
}

pub struct Row<'a> {
    constraints: &'a [Constraint],
    gap: u16,
}

impl<'a> Row<'a> {
    pub fn new(constraints: &'a [Constraint]) -> Self {
        Self {
            constraints,
            gap: 0,
        }
    }

    pub fn gap(mut self, gap: u16) -> Self {
        self.gap = gap;
        self
    }

    pub fn split(&self, area: Rect) -> Vec<Rect> {
        split(area, self.constraints, self.gap, true)
    }
}

pub struct Column<'a> {
    constraints: &'a [Constraint],
    gap: u16,
}

impl<'a> Column<'a> {
    pub fn new(constraints: &'a [Constraint]) -> Self {
        Self {
            constraints,
            gap: 0,
        }
    }

    pub fn gap(mut self, gap: u16) -> Self {
        self.gap = gap;
        self
    }

    pub fn split(&self, area: Rect) -> Vec<Rect> {
        split(area, self.constraints, self.gap, false)
    }
}

fn split(area: Rect, constraints: &[Constraint], gap: u16, horizontal: bool) -> Vec<Rect> {
    if constraints.is_empty() {
        return Vec::new();
    }
    let total = if horizontal { area.width } else { area.height };
    let gaps = gap.saturating_mul(constraints.len().saturating_sub(1) as u16);
    let available = total.saturating_sub(gaps);
    let mut used = 0usize;
    let mut fills = 0usize;
    for constraint in constraints {
        match constraint {
            Constraint::Fixed(length) => used += *length as usize,
            Constraint::Ratio(numerator, denominator) => {
                used += (available as u32 * numerator / denominator.max(&1)) as usize
            }
            Constraint::Fill => fills += 1,
        }
    }
    let remaining = (available as usize).saturating_sub(used);
    let mut offset = 0usize;
    let mut fill_index = 0usize;
    constraints
        .iter()
        .map(|constraint| {
            if offset > 0 {
                offset += gap as usize;
            }
            let length = match constraint {
                Constraint::Fixed(length) => (*length as usize).min(available as usize),
                Constraint::Ratio(numerator, denominator) => {
                    (available as u32 * numerator / denominator.max(&1)) as usize
                }
                Constraint::Fill => {
                    fill_index += 1;
                    let share = remaining / fills.max(1);
                    if fill_index == fills {
                        remaining - share * (fills - 1)
                    } else {
                        share
                    }
                }
            }
            .min((total as usize).saturating_sub(offset));
            let rect = if horizontal {
                Rect::new(area.x + offset as u16, area.y, length as u16, area.height)
            } else {
                Rect::new(area.x, area.y + offset as u16, area.width, length as u16)
            };
            offset += length;
            rect
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_and_column_honour_fixed_ratio_fill_and_gap() {
        let row = Row::new(&[
            Constraint::Fixed(2),
            Constraint::Ratio(1, 2),
            Constraint::Fill,
        ])
        .gap(1);
        let parts = row.split(Rect::new(0, 0, 14, 1));
        assert_eq!(parts[0], Rect::new(0, 0, 2, 1));
        assert_eq!(parts[1], Rect::new(3, 0, 6, 1));
        assert_eq!(parts[2], Rect::new(10, 0, 4, 1));
        let parts =
            Column::new(&[Constraint::Fill, Constraint::Fixed(2)]).split(Rect::new(0, 0, 4, 5));
        assert_eq!(parts[0].height, 3);
        assert_eq!(parts[1].y, 3);
        assert_eq!(parts[1].height, 2);
        assert!(Row::new(&[]).split(Rect::default()).is_empty());
    }
}
