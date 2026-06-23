-- Agent capability scope (Layer B), split into a project ceiling and a
-- per-member grant. Both store a JSON array of capability names (e.g.
-- ["automation.read", "member.manage"]); NULL means "unset":
--   projects.agent_capability_ceiling NULL  -> no project-level limit (all)
--   project_members.agent_capabilities NULL -> inherit the project ceiling
-- Effective agent policy = (member grant ?? ceiling) ∩ ceiling.
ALTER TABLE projects ADD COLUMN agent_capability_ceiling TEXT;
ALTER TABLE project_members ADD COLUMN agent_capabilities TEXT;

-- The owner's own grant lives in the same place as every other member's, so
-- make the owner a first-class project_members row (no special owner column or
-- branch). Backfill existing projects whose owner has no row yet; new projects
-- insert the owner row in create_project.
-- INSERT INTO project_members (project_id, user_id, added_at)
-- SELECT p.id, p.owner_id, p.created_at
-- FROM projects p
-- WHERE NOT EXISTS (
--     SELECT 1 FROM project_members pm
--     WHERE pm.project_id = p.id AND pm.user_id = p.owner_id
-- );
