SELECT
  (sum("sales"."amount") / count("sales"."units")) AS "text"
FROM "sales"
