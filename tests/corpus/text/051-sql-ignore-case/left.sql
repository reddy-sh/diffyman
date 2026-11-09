select id, name from customers
where city = 'Été' and active = 1
order by name;

update streets set name = 'Straße' where id = 7;
delete from sessions where expires < now();
