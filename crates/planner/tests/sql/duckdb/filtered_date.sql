SELECT
  "sales"."region" AS "x",
  sum("sales"."amount") AS "y"
FROM "sales"
WHERE ("sales"."month" >= CAST(? AS DATE))
GROUP BY 1
