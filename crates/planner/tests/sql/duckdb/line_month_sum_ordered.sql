SELECT
  "sales"."month" AS "x",
  sum("sales"."amount") AS "y"
FROM "sales"
GROUP BY 1
ORDER BY "x" ASC
