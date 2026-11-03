    use super::*;

    #[test]
    fn builds_sql_server_and_oracle_odbc_connection_strings() {
        let mut sqlserver = ConnectionConfig::new("sql", "sqlsrv:Server=db;Database=app;Encrypt=yes").unwrap();
        sqlserver.username = Some("user".into());
        sqlserver.password = Some("p;ass".into());
        let sqlserver = unsafe { CStr::from_ptr(odbc_connection_string(&sqlserver).unwrap().as_ptr()).to_string_lossy().into_owned() };
        assert!(sqlserver.contains("DRIVER={ODBC Driver 18 for SQL Server}"));
        assert!(sqlserver.contains("SERVER={db}"));
        assert!(sqlserver.contains("PWD={p;ass}"));
        let mut oracle = ConnectionConfig::new("ora", "oci:dbname=//db:1521/XEPDB1").unwrap();
        oracle.driver = Some("Oracle in OraDB".into());
        let oracle = unsafe { CStr::from_ptr(odbc_connection_string(&oracle).unwrap().as_ptr()).to_string_lossy().into_owned() };
        assert!(oracle.contains("DRIVER={Oracle in OraDB}"));
        assert!(oracle.contains("DBQ=//db:1521/XEPDB1"));
        assert!(!oracle.contains("DBQ={//db:1521/XEPDB1}"));
    }

    #[test]
    fn odbc_parameter_interpolation_escapes_quotes_and_preserves_unicode() {
        let sql = "SELECT ?, 'literal ?', /* ? */ ?";
        let actual = interpolate(sql, &[DbValue::String("O'Neil café".into()), DbValue::Integer(4)], DatabaseKind::SqlServer).unwrap();
        assert_eq!(actual, "SELECT 'O''Neil café', 'literal ?', /* ? */ 4");
        assert!(interpolate("SELECT ?", &[], DatabaseKind::SqlServer).is_err());
    }
