SELECT
  "sales"."region" AS "x",
  sum("sales"."amount") AS "y",
  "sales"."month" AS "color"
FROM "sales"
GROUP BY 1, 3
