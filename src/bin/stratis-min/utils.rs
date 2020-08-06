#[macro_export]
macro_rules! left_align {
    ($string:expr, $max_length:expr) => {{
        let len = $string.len();
        $string + vec![" "; $max_length - len + 3].join("").as_str()
    }};
}

#[macro_export]
macro_rules! right_align {
    ($string:expr, $max_length:expr) => {
        vec![" "; $max_length - $string.len() + 3].join("") + $string.as_str()
    };
}

#[macro_export]
macro_rules! align {
    ($string:expr, $max_length:expr, $align:tt) => {
        if $align == ">" {
            $crate::right_align!($string, $max_length)
        } else {
            $crate::left_align!($string, $max_length)
        }
    };
}

#[macro_export]
macro_rules! print_table {
    ($($heading:expr, $values:expr, $align:tt);*) => {{
        let (lengths_same, lengths) = vec![$($values.len()),*]
            .into_iter()
            .fold((true, None), |(is_same, len_opt), len| {
                if len_opt.is_none() {
                    (true, Some(len))
                } else {
                    (is_same && len_opt == Some(len), len_opt)
                }
            });
        if !lengths_same {
            return Err(libstratis::stratis::StratisError::Error(
                "All values parameters must be the same length".to_string()
            ));
        }
        let mut output = vec![String::new(); lengths.unwrap_or(0) + 1];
        $(
            let max_length = $values
                .iter()
                .fold($heading.len(), |acc, val| {
                    if val.len() > acc {
                        val.len()
                    } else {
                        acc
                    }
                });
            if let Some(string) = output.get_mut(0) {
                string.push_str($crate::align!($heading.to_string(), max_length, $align).as_str());
            }
            for (index, row_seg) in $values.into_iter()
                .map(|s| $crate::align!(s, max_length, $align))
                .enumerate()
            {
                if let Some(string) = output.get_mut(index + 1) {
                    string.push_str(row_seg.as_str());
                }
            }
        )*
        for row in output.into_iter() {
            println!("{}", row);
        }
    }};
}
