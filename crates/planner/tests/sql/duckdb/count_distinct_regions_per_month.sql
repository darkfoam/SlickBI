SELECT
  "sales"."month" AS "x",
  count(DISTINCT "sales"."region") AS "y"
FROM "sales"
GROUP BY 1
