SELECT
  "sales"."region" AS "x",
  sum("sales"."amount") AS "y"
FROM "sales"
WHERE ("sales"."region" IN (?, ?))
GROUP BY 1
