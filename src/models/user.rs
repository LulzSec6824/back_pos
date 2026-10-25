pub enum EmployeeType {
    CASHIER,
    KEEPER,
    LOGISTIC,
    MANAGER,
}

pub struct StoreInfo {
    address: String,
    employee_id: i32,
    employee_type: EmployeeType,
}
