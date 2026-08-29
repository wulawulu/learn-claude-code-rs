use calculator::divide;

#[test]
fn divides_nonzero_numbers() {
    assert_eq!(divide(10, 2), 5);
}

#[test]
fn divide_by_zero_returns_zero() {
    assert_eq!(divide(10, 0), 0);
}
