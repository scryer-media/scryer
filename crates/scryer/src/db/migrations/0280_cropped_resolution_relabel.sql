-- Relabel stored resolution tiers with the crop-tolerant dimension ladder.
-- Imports and analysis wrote the tier derived from the decoded video size into
-- both `resolution` and `quality_id`. The earlier ladder demanded near-full
-- frame widths, so cropped encodes were stored a tier or more too low (a
-- 1918x802 file as 720p, a 3832x1600 file as 1440p), and upgrade decisions
-- read those stored labels. Only rows with positive dimensions are touched,
-- and only bare tier labels are rewritten: any other stored value, such as an
-- interlaced or compound label, is kept. An empty `resolution` is filled from
-- the dimensions; an empty `quality_id` is left alone.
UPDATE media_files
   SET resolution = CASE
           WHEN video_width >= 7680 OR video_height >= 4200 THEN '4320p'
           WHEN video_width >= 3200 OR video_height >= 2100 THEN '2160p'
           WHEN video_width >= 2400 OR video_height >= 1300 THEN '1440p'
           WHEN video_width >= 1800 OR video_height >= 1000 THEN '1080p'
           WHEN video_width >= 1200 OR video_height >= 700 THEN '720p'
           WHEN video_width >= 1000 OR video_height >= 560 THEN '576p'
           ELSE '480p'
       END
 WHERE video_width > 0
   AND video_height > 0
   AND lower(trim(COALESCE(resolution, ''))) IN
       ('', '4320p', '2160p', '1440p', '1080p', '720p', '576p', '480p', '360p')
   AND lower(trim(COALESCE(resolution, ''))) <> CASE
           WHEN video_width >= 7680 OR video_height >= 4200 THEN '4320p'
           WHEN video_width >= 3200 OR video_height >= 2100 THEN '2160p'
           WHEN video_width >= 2400 OR video_height >= 1300 THEN '1440p'
           WHEN video_width >= 1800 OR video_height >= 1000 THEN '1080p'
           WHEN video_width >= 1200 OR video_height >= 700 THEN '720p'
           WHEN video_width >= 1000 OR video_height >= 560 THEN '576p'
           ELSE '480p'
       END;

UPDATE media_files
   SET quality_id = CASE
           WHEN video_width >= 7680 OR video_height >= 4200 THEN '4320p'
           WHEN video_width >= 3200 OR video_height >= 2100 THEN '2160p'
           WHEN video_width >= 2400 OR video_height >= 1300 THEN '1440p'
           WHEN video_width >= 1800 OR video_height >= 1000 THEN '1080p'
           WHEN video_width >= 1200 OR video_height >= 700 THEN '720p'
           WHEN video_width >= 1000 OR video_height >= 560 THEN '576p'
           ELSE '480p'
       END
 WHERE video_width > 0
   AND video_height > 0
   AND lower(trim(COALESCE(quality_id, ''))) IN
       ('4320p', '2160p', '1440p', '1080p', '720p', '576p', '480p', '360p')
   AND lower(trim(COALESCE(quality_id, ''))) <> CASE
           WHEN video_width >= 7680 OR video_height >= 4200 THEN '4320p'
           WHEN video_width >= 3200 OR video_height >= 2100 THEN '2160p'
           WHEN video_width >= 2400 OR video_height >= 1300 THEN '1440p'
           WHEN video_width >= 1800 OR video_height >= 1000 THEN '1080p'
           WHEN video_width >= 1200 OR video_height >= 700 THEN '720p'
           WHEN video_width >= 1000 OR video_height >= 560 THEN '576p'
           ELSE '480p'
       END;
