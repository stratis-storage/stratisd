extern crate stratisd;

#[cfg(test)]
mod tests {
    #[test]
    fn test_true() {
        assert!(true);
    }

    #[test]
    #[should_panic]
    fn test_false() {
        assert!(false);
    }
}
