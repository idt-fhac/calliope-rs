\* Source Pyomo model name=None *\

min 
objectives(min_cost_optimisation)(0):
+0.00025152527897265027 variables(flow_cap)(a__test_supply_elec__electricity)

s.t.

c_u_constraints(flow_in_max)(a__test_demand_elec__electricity__2005_01_01_00_00)_:
+1 variables(flow_in)(a__test_demand_elec__electricity__2005_01_01_00_00)
-1.0 variables(flow_cap)(a__test_demand_elec__electricity)
<= 0.0

c_u_constraints(flow_in_max)(a__test_demand_elec__electricity__2005_01_01_01_00)_:
-1.0 variables(flow_cap)(a__test_demand_elec__electricity)
+1 variables(flow_in)(a__test_demand_elec__electricity__2005_01_01_01_00)
<= 0.0

c_u_constraints(source_max)(a__test_supply_elec__2005_01_01_00_00)_:
+1 variables(source_use)(a__test_supply_elec__2005_01_01_00_00)
-1.0 variables(source_cap)(a__test_supply_elec)
<= 0.0

c_u_constraints(source_max)(a__test_supply_elec__2005_01_01_01_00)_:
-1.0 variables(source_cap)(a__test_supply_elec)
+1 variables(source_use)(a__test_supply_elec__2005_01_01_01_00)
<= 0.0

c_e_constraints(system_balance)(a__electricity__2005_01_01_00_00)_:
-1 variables(flow_in)(a__test_demand_elec__electricity__2005_01_01_00_00)
+1 variables(flow_out)(a__test_supply_elec__electricity__2005_01_01_00_00)
= 0.0

c_e_constraints(system_balance)(a__electricity__2005_01_01_01_00)_:
-1 variables(flow_in)(a__test_demand_elec__electricity__2005_01_01_01_00)
+1 variables(flow_out)(a__test_supply_elec__electricity__2005_01_01_01_00)
= 0.0

c_e_constraints(balance_demand)(a__test_demand_elec__electricity__2005_01_01_00_00)_:
+1 variables(flow_in)(a__test_demand_elec__electricity__2005_01_01_00_00)
= 5.0

c_e_constraints(balance_demand)(a__test_demand_elec__electricity__2005_01_01_01_00)_:
+1 variables(flow_in)(a__test_demand_elec__electricity__2005_01_01_01_00)
= 5.0

c_e_constraints(balance_supply_no_storage)(a__test_supply_elec__electricity__2005_01_01_00_00)_:
-1 variables(source_use)(a__test_supply_elec__2005_01_01_00_00)
+1.1111111111111112 variables(flow_out)(a__test_supply_elec__electricity__2005_01_01_00_00)
= 0.0

c_e_constraints(balance_supply_no_storage)(a__test_supply_elec__electricity__2005_01_01_01_00)_:
-1 variables(source_use)(a__test_supply_elec__2005_01_01_01_00)
+1.1111111111111112 variables(flow_out)(a__test_supply_elec__electricity__2005_01_01_01_00)
= 0.0

c_u_constraints(unit_commitment_milp)(a__test_supply_elec__2005_01_01_00_00)_:
+1 variables(operating_units)(a__test_supply_elec__2005_01_01_00_00)
-1 variables(purchased_units)(a__test_supply_elec)
<= 0.0

c_u_constraints(unit_commitment_milp)(a__test_supply_elec__2005_01_01_01_00)_:
-1 variables(purchased_units)(a__test_supply_elec)
+1 variables(operating_units)(a__test_supply_elec__2005_01_01_01_00)
<= 0.0

c_u_constraints(flow_out_max_milp)(a__test_supply_elec__electricity__2005_01_01_00_00)_:
+1 variables(flow_out)(a__test_supply_elec__electricity__2005_01_01_00_00)
-15.0 variables(operating_units)(a__test_supply_elec__2005_01_01_00_00)
<= 0.0

c_u_constraints(flow_out_max_milp)(a__test_supply_elec__electricity__2005_01_01_01_00)_:
+1 variables(flow_out)(a__test_supply_elec__electricity__2005_01_01_01_00)
-15.0 variables(operating_units)(a__test_supply_elec__2005_01_01_01_00)
<= 0.0

c_e_constraints(flow_capacity_units_milp)(a__test_supply_elec__electricity)_:
+1 variables(flow_cap)(a__test_supply_elec__electricity)
-15.0 variables(purchased_units)(a__test_supply_elec)
= 0.0

c_u_constraints(flow_capacity_max_purchase_milp)(a__test_supply_elec__electricity)_:
+1 variables(flow_cap)(a__test_supply_elec__electricity)
-1000.0 variables(purchased_units)(a__test_supply_elec)
<= 0.0

bounds
   0.0 <= variables(flow_cap)(a__test_supply_elec__electricity) <= +inf
   0.0 <= variables(flow_in)(a__test_demand_elec__electricity__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_cap)(a__test_demand_elec__electricity) <= +inf
   0.0 <= variables(flow_in)(a__test_demand_elec__electricity__2005_01_01_01_00) <= +inf
   0.0 <= variables(source_use)(a__test_supply_elec__2005_01_01_00_00) <= +inf
   0.0 <= variables(source_cap)(a__test_supply_elec) <= +inf
   0.0 <= variables(source_use)(a__test_supply_elec__2005_01_01_01_00) <= +inf
   0.0 <= variables(flow_out)(a__test_supply_elec__electricity__2005_01_01_00_00) <= +inf
   0.0 <= variables(flow_out)(a__test_supply_elec__electricity__2005_01_01_01_00) <= +inf
   0.0 <= variables(operating_units)(a__test_supply_elec__2005_01_01_00_00) <= +inf
   0.0 <= variables(purchased_units)(a__test_supply_elec) <= 1.0
   0.0 <= variables(operating_units)(a__test_supply_elec__2005_01_01_01_00) <= +inf
general
  variables(operating_units)(a__test_supply_elec__2005_01_01_00_00)
  variables(operating_units)(a__test_supply_elec__2005_01_01_01_00)
binary
  variables(purchased_units)(a__test_supply_elec)
end
