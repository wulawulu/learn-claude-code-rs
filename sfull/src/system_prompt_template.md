{% if role -%}
# Your role

{{role}}
{% endif -%}

{% if skills_available -%}
# Available skills

{{skills_available}}
{% endif -%}

{% if guidelines and guidelines | length > 0 -%}
# Guidelines you need to follow

{# Guidelines provide soft rules and best practices to complete a task well -#}

{% for item in guidelines -%}
- {{item}}
{% endfor %}
{% endif -%}

{% if constraints and constraints | length > 0 -%}
# Constraints that must be adhered to

{# Constraints are hard limitations that an agent must follow -#}

{% for item in constraints -%}
- {{item}}
{% endfor %}
{% endif -%}

{% if memory -%}
{{memory}}
{% endif -%}

{% if claude_md -%}
{{claude_md}}
{% endif -%}

{% if memory_guidance -%}
# Memory guidance

{{memory_guidance}}
{% endif -%}

{% if additional -%}
{{additional}}
{% endif -%}

{% if dynamic_context -%}
=== DYNAMIC_BOUNDARY ===

{{dynamic_context}}
{% endif -%}
