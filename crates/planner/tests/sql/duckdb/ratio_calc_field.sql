SELECT
  "sales"."region" AS "x",
  sum(("sales"."amount" * ?)) AS "y"
FROM "sales"
GROUP BY 1
