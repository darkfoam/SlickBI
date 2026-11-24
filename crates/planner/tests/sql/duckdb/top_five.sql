SELECT
  "sales"."region" AS "x",
  sum("sales"."amount") AS "y"
FROM "sales"
GROUP BY 1
ORDER BY "y" DESC
LIMIT 5
