BEGIN TRANSACTION;
CREATE TABLE transactions (Account INTEGER, Description TEXT, Day DATE, Type TEXT, Amount REAL, Balance REAL, Derived REAL, Note TEXT);
INSERT INTO transactions VALUES (1001, 'Coffee shop', '2025-01-02', 'Debit', -4.25, 1245.75, 4.25, 'A-1');
INSERT INTO transactions VALUES (1001, 'Payroll', '2025-01-03', 'Credit', 2000.0, 3245.75, -2000.0, 'B-2');
INSERT INTO transactions VALUES (1001, 'Rent', '2025-01-04', 'Debit', -1200.0, 2045.75, 1200.0, 'C-3');
COMMIT;
